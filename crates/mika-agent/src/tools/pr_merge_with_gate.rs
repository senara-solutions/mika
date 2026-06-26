use std::sync::LazyLock;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use super::{Tool, ToolContext, ToolOutput};
use crate::async_db::AsyncDatabase;

/// Maximum bytes to read from a `gh` subprocess stdout/stderr (256 KB).
const MAX_OUTPUT_LEN: usize = 256 * 1024;

/// Allowed merge methods — passed as `--{method}` to `gh pr merge`.
const ALLOWED_MERGE_METHODS: &[&str] = &["squash", "merge", "rebase"];

/// Regex for validating `owner/repo` format.
static REPO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$").unwrap());

// ---------------------------------------------------------------------------
// Public tool struct
// ---------------------------------------------------------------------------

pub struct PrMergeWithGateTool;

#[async_trait]
impl Tool for PrMergeWithGateTool {
    fn name(&self) -> &str {
        "pr_merge_with_gate"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "pr_merge_with_gate".to_string(),
            description: "Merge a GitHub pull request with a CI gate. Checks the status of \
                required CI checks before merging. If any required check is failing, the merge \
                is blocked and the failing checks are returned. If checks are still pending \
                (but none failing), auto-merge is enabled so GitHub merges automatically when \
                checks pass. If all required checks pass, the PR is merged immediately.\n\n\
                IMPORTANT: 'auto_merge_enabled' means GitHub will merge when all checks pass — \
                the PR is NOT yet merged. Do not claim the PR is merged until you confirm it.\n\n\
                After a successful merge (action: 'merged'), update the task status before \
                reporting to the user.\n\n\
                Returns a structured JSON response with an 'action' field. Possible actions: \
                'merged', 'auto_merge_enabled', 'blocked', 'already_merged', 'gate_errored'. \
                Branch on 'action' to determine next steps."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr_number": {
                        "type": "integer",
                        "description": "Pull request number"
                    },
                    "repo": {
                        "type": "string",
                        "description": "Repository in owner/repo format (e.g. 'senara-solutions/mika')"
                    },
                    "merge_method": {
                        "type": "string",
                        "description": "Merge method: 'squash' (default), 'merge', or 'rebase'",
                        "enum": ["squash", "merge", "rebase"],
                        "default": "squash"
                    },
                    "delete_branch": {
                        "type": "boolean",
                        "description": "Delete head branch after merge (default: true)",
                        "default": true
                    }
                },
                "required": ["pr_number", "repo"],
                "additionalProperties": false
            }),
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(60)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // -- Extract and validate inputs --
        let pr_number = match input.get("pr_number").and_then(|v| v.as_u64()) {
            Some(n) if n > 0 => n,
            _ => {
                return Ok(ToolOutput::error(
                    "pr_number is required and must be a positive integer",
                ));
            }
        };

        let repo = match input.get("repo").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return Ok(ToolOutput::error("repo is required (owner/repo format)")),
        };
        if let Err(e) = validate_repo(repo) {
            return Ok(ToolOutput::error(e));
        }

        let merge_method = input
            .get("merge_method")
            .and_then(|v| v.as_str())
            .unwrap_or("squash");
        if let Err(e) = validate_merge_method(merge_method) {
            return Ok(ToolOutput::error(e));
        }

        let delete_branch = input
            .get("delete_branch")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // -- Require GitHub token --
        let token = match ctx.github_token {
            Some(t) => t,
            None => {
                return Ok(ToolOutput::error(
                    "GitHub token required for pr_merge_with_gate. \
                     Set MIKA_GITHUB_TOKEN or configure a GitHub App.",
                ));
            }
        };

        // -- Step 1: Preflight — check PR state via gh pr view --
        let preflight = match run_gh_pr_view(pr_number, repo, token).await {
            Ok(pf) => pf,
            Err(e) => {
                let result = MergeGateResult::GateError {
                    kind: GateErrorKind::GhCliFailure {
                        exit_code: e.exit_code.unwrap_or(-1),
                    },
                    detail: e.message,
                };
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
            }
        };

        // Classify preflight result
        if let Some(result) = classify_preflight(&preflight) {
            return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
        }

        // -- Step 1b: Behind-main assertion (#1577) --
        // Block merge if PR base is behind the current main HEAD.
        // Fail-open: API errors log a warning and proceed.
        match is_behind_main(&preflight.base_ref_oid, repo, token).await {
            Ok(Some(info)) => {
                let detail = format!(
                    "PR is behind main — rebase needed. \
                     PR base: {}, main HEAD: {}",
                    info.pr_base_sha, info.current_main_sha
                );
                let result = MergeGateResult::Blocked {
                    reason: BlockReason::BehindMain {
                        pr_base_sha: info.pr_base_sha,
                        current_main_sha: info.current_main_sha,
                    },
                    failing_checks: vec![],
                    detail,
                };
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
            }
            Ok(None) => {} // Up-to-date — proceed
            Err(e) => {
                warn!(
                    pr_number,
                    error = %e,
                    "Failed to check behind-main status — proceeding with merge (fail-open)"
                );
            }
        }

        // -- Step 2: Fetch required check statuses --
        let checks_result = run_gh_checks(pr_number, repo, token).await;
        let checks = match checks_result {
            Ok(c) => c,
            Err(e) => {
                let result = MergeGateResult::GateError {
                    kind: GateErrorKind::GhCliFailure {
                        exit_code: parse_exit_code_from_error(&e),
                    },
                    detail: format!("Failed to fetch check statuses: {e}"),
                };
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
            }
        };

        // -- Step 3: Classify and act --
        let classification = classify_checks(&checks);

        match classification {
            CheckClassification::HasFailures => {
                let failing: Vec<CheckInfo> = checks
                    .iter()
                    .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                    .map(|c| CheckInfo {
                        name: c.name.clone(),
                        state: c.state.clone(),
                        link: c.link.clone(),
                    })
                    .collect();

                let result = MergeGateResult::Blocked {
                    reason: BlockReason::RequiredCheckFailed {
                        failing_checks: failing.clone(),
                    },
                    failing_checks: failing,
                    detail: format!(
                        "{} required check(s) failed",
                        checks
                            .iter()
                            .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                            .count()
                    ),
                };
                Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
            }
            CheckClassification::HasPending => {
                // Enable auto-merge — GitHub merges when checks pass
                let auto_result =
                    run_gh_merge(pr_number, repo, merge_method, delete_branch, true, token).await;

                match auto_result {
                    Ok(_output) => {
                        // mika#1211: persist pr_url on supervisor so the orphan
                        // reaper (#871) doesn't flip it to `failed` and the
                        // parent-completer (mika#1162) can promote it to
                        // `completed` once the dispatch callback ages past
                        // REAPER_GRACE_SECONDS. Mirrors the metadata-write
                        // pattern in dispatcher::try_extract_callback_metadata.
                        let pr_url = format!("https://github.com/{repo}/pull/{pr_number}");
                        write_auto_merge_pr_url_to_supervisor(ctx, &pr_url).await;

                        let pending: Vec<CheckInfo> = checks
                            .iter()
                            .filter(|c| c.bucket == "pending")
                            .map(|c| CheckInfo {
                                name: c.name.clone(),
                                state: c.state.clone(),
                                link: None,
                            })
                            .collect();

                        let result = MergeGateResult::AutoMergeEnabled {
                            pending_checks: pending,
                        };
                        Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                    }
                    Err(e) => {
                        let result = MergeGateResult::GateError {
                            kind: GateErrorKind::GhCliFailure {
                                exit_code: parse_exit_code_from_error(&e),
                            },
                            detail: format!("Auto-merge failed: {e}"),
                        };
                        Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                    }
                }
            }
            CheckClassification::AllPassed => {
                // Merge immediately
                let merge_result =
                    run_gh_merge(pr_number, repo, merge_method, delete_branch, false, token).await;

                // Unify success/error into a single string for "already merged" detection
                let (output, is_err) = match merge_result {
                    Ok(s) => (s, false),
                    Err(s) => (s, true),
                };

                let output_lower = output.to_lowercase();
                if output_lower.contains("already been merged") {
                    let result = MergeGateResult::AlreadyMerged;
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if !is_err {
                    let result = MergeGateResult::Merged;
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if output_lower.contains("draft") {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::Draft,
                        failing_checks: vec![],
                        detail: "PR is a draft — convert to ready before merging".to_string(),
                    };
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if output_lower.contains("merge conflict")
                    || output_lower.contains("not mergeable")
                {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::MergeConflict,
                        failing_checks: vec![],
                        detail: "PR has merge conflicts — rebase needed".to_string(),
                    };
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else if output_lower.contains("review") && output_lower.contains("required") {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::MissingApproval,
                        failing_checks: vec![],
                        detail: "Required reviews not met".to_string(),
                    };
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                } else {
                    let result = MergeGateResult::GateError {
                        kind: GateErrorKind::Unknown,
                        detail: format!("Merge failed: {output}"),
                    };
                    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Structured result returned by the tool as JSON.
/// Tagged union — the LLM branches on the `action` field.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "action")]
pub(crate) enum MergeGateResult {
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "auto_merge_enabled")]
    AutoMergeEnabled { pending_checks: Vec<CheckInfo> },
    #[serde(rename = "blocked")]
    Blocked {
        reason: BlockReason,
        /// Retained for backward compatibility — existing prompts branch on this field.
        failing_checks: Vec<CheckInfo>,
        detail: String,
    },
    #[serde(rename = "already_merged")]
    AlreadyMerged,
    #[serde(rename = "gate_errored")]
    GateError { kind: GateErrorKind, detail: String },
}

/// Why a PR is blocked from merging.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "reason")]
pub(crate) enum BlockReason {
    /// PR is CONFLICTING or mergeStateStatus is DIRTY.
    #[serde(rename = "merge_conflict")]
    MergeConflict,
    /// One or more required CI checks failed.
    #[serde(rename = "required_check_failed")]
    RequiredCheckFailed { failing_checks: Vec<CheckInfo> },
    /// Branch protection requires approval reviews.
    #[serde(rename = "missing_approval")]
    MissingApproval,
    /// PR is closed (not merged).
    #[serde(rename = "pr_closed")]
    PrClosed,
    /// PR is a draft.
    #[serde(rename = "draft")]
    Draft,
    /// PR base is behind the current main HEAD — rebase needed before merge.
    #[serde(rename = "behind_main")]
    BehindMain {
        pr_base_sha: String,
        current_main_sha: String,
    },
}

/// Why the gate tool itself failed (infrastructure, not PR state).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind")]
#[allow(dead_code)] // Variants are part of the exhaustive error taxonomy; not all constructed yet.
pub(crate) enum GateErrorKind {
    /// gh CLI returned non-zero exit code.
    #[serde(rename = "gh_cli_failure")]
    GhCliFailure { exit_code: i32 },
    /// Network-level failure.
    #[serde(rename = "network_error")]
    NetworkError,
    /// Failed to parse gh output.
    #[serde(rename = "parse_error")]
    ParseError,
    /// Unclassified failure.
    #[serde(rename = "unknown")]
    Unknown,
}

/// Check info included in the structured result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CheckInfo {
    pub(crate) name: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) link: Option<String>,
}

/// A single check from `gh pr checks --json` output.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct GhCheck {
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) bucket: String,
    #[serde(default)]
    pub(crate) link: Option<String>,
}

/// Classification of the overall check status.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CheckClassification {
    /// All checks passed (or no required checks exist).
    AllPassed,
    /// Some checks are pending, but none have failed.
    HasPending,
    /// At least one check has failed or been cancelled.
    HasFailures,
}

/// Preflight PR state from `gh pr view`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrPreflight {
    pub(crate) mergeable: String,
    pub(crate) merge_state_status: String,
    #[serde(default)]
    pub(crate) is_draft: bool,
    pub(crate) state: String,
    /// The SHA of the base branch commit this PR was created against.
    /// Used by the behind-main assertion (#1577) to detect stale PRs.
    #[serde(default)]
    pub(crate) base_ref_oid: String,
}

/// Error from `run_gh_pr_view` with optional exit code.
#[derive(Debug)]
pub(crate) struct PreflightError {
    pub(crate) message: String,
    pub(crate) exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_repo(repo: &str) -> Result<(), String> {
    if repo.is_empty() {
        return Err("repo cannot be empty".to_string());
    }
    if repo.len() > 200 {
        return Err("repo is too long (max 200 characters)".to_string());
    }
    if !REPO_RE.is_match(repo) {
        return Err(format!(
            "Invalid repo format: '{repo}'. Expected owner/repo (e.g. 'senara-solutions/mika')"
        ));
    }
    Ok(())
}

fn validate_merge_method(method: &str) -> Result<(), String> {
    if ALLOWED_MERGE_METHODS.contains(&method) {
        Ok(())
    } else {
        Err(format!(
            "Invalid merge_method: '{method}'. Allowed: {}",
            ALLOWED_MERGE_METHODS.join(", ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Preflight classification (pure function — easily testable)
// ---------------------------------------------------------------------------

/// Classify preflight PR state into an immediate result, or None if checks
/// should proceed (PR is open, not draft, not conflicting).
pub(crate) fn classify_preflight(preflight: &PrPreflight) -> Option<MergeGateResult> {
    let state_upper = preflight.state.to_uppercase();

    // Already merged — primary detection path (before merge attempt)
    if state_upper == "MERGED" {
        return Some(MergeGateResult::AlreadyMerged);
    }

    // Closed without merge
    if state_upper == "CLOSED" {
        return Some(MergeGateResult::Blocked {
            reason: BlockReason::PrClosed,
            failing_checks: vec![],
            detail: "PR is closed".to_string(),
        });
    }

    // Draft PR
    if preflight.is_draft {
        return Some(MergeGateResult::Blocked {
            reason: BlockReason::Draft,
            failing_checks: vec![],
            detail: "PR is a draft — convert to ready before merging".to_string(),
        });
    }

    // Merge conflict detection — the #792 fix
    let mergeable_upper = preflight.mergeable.to_uppercase();
    let merge_state_upper = preflight.merge_state_status.to_uppercase();
    if mergeable_upper == "CONFLICTING" || merge_state_upper == "DIRTY" {
        return Some(MergeGateResult::Blocked {
            reason: BlockReason::MergeConflict,
            failing_checks: vec![],
            detail: "PR has merge conflicts — rebase needed".to_string(),
        });
    }

    // No immediate blocker — proceed to check classification
    None
}

// ---------------------------------------------------------------------------
// Check classification (pure function — easily testable)
// ---------------------------------------------------------------------------

pub(crate) fn classify_checks(checks: &[GhCheck]) -> CheckClassification {
    let has_failures = checks
        .iter()
        .any(|c| matches!(c.bucket.as_str(), "fail" | "cancel"));
    let has_pending = checks.iter().any(|c| c.bucket == "pending");

    if has_failures {
        CheckClassification::HasFailures
    } else if has_pending {
        CheckClassification::HasPending
    } else {
        CheckClassification::AllPassed
    }
}

// ---------------------------------------------------------------------------
// Subprocess helpers
// ---------------------------------------------------------------------------

/// Run `gh pr view <number> --repo <repo> --json mergeable,mergeStateStatus,isDraft,state`
/// and return the parsed preflight struct.
pub(crate) async fn run_gh_pr_view(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<PrPreflight, PreflightError> {
    let pr_str = pr_number.to_string();
    let args = vec![
        "pr",
        "view",
        &pr_str,
        "--repo",
        repo,
        "--json",
        "mergeable,mergeStateStatus,isDraft,state,baseRefOid",
    ];

    let output = run_gh_subprocess(&args, token).await.map_err(|e| {
        let exit_code = parse_exit_code_from_error(&e);
        PreflightError {
            message: e,
            exit_code: Some(exit_code),
        }
    })?;

    serde_json::from_str::<PrPreflight>(output.trim()).map_err(|e| PreflightError {
        message: format!("Failed to parse gh pr view output: {e}"),
        exit_code: None,
    })
}

/// Run `gh pr checks <number> --repo <repo> --required --json name,state,bucket,link`
/// and return the parsed check list.
pub(crate) async fn run_gh_checks(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<GhCheck>, String> {
    let pr_str = pr_number.to_string();
    let args = vec![
        "pr",
        "checks",
        &pr_str,
        "--repo",
        repo,
        "--required",
        "--json",
        "name,state,bucket,link",
    ];

    let output = run_gh_subprocess(&args, token).await?;

    // Empty output or "[]" means no required checks
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }

    serde_json::from_str::<Vec<GhCheck>>(trimmed)
        .map_err(|e| format!("Failed to parse gh pr checks output: {e}"))
}

// ---------------------------------------------------------------------------
// Behind-main detection (#1577)
// ---------------------------------------------------------------------------

/// Info returned when a PR is behind main.
#[derive(Debug, Clone)]
pub(crate) struct BehindMainInfo {
    pub(crate) pr_base_sha: String,
    pub(crate) current_main_sha: String,
}

/// Fetch the current HEAD SHA of the default branch (`main`) via the GitHub API.
///
/// Uses `gh api repos/{repo}/git/ref/heads/main --jq .object.sha`.
pub(crate) async fn fetch_main_head_sha(repo: &str, token: &str) -> Result<String, String> {
    let endpoint = format!("repos/{repo}/git/ref/heads/main");
    let args = vec!["api", &endpoint, "--jq", ".object.sha"];

    let output = run_gh_subprocess(&args, token).await?;
    let sha = output.trim().to_string();
    if sha.is_empty() {
        return Err("Empty SHA returned from GitHub API for main HEAD".to_string());
    }
    Ok(sha)
}

/// Check whether a PR is behind `main` by comparing its `baseRefOid` against
/// the current main HEAD SHA.
///
/// Returns `Ok(Some(info))` when behind, `Ok(None)` when up-to-date,
/// `Err` on API failure.
pub(crate) async fn is_behind_main(
    base_ref_oid: &str,
    repo: &str,
    token: &str,
) -> Result<Option<BehindMainInfo>, String> {
    let current_main_sha = fetch_main_head_sha(repo, token).await?;

    if base_ref_oid != current_main_sha {
        Ok(Some(BehindMainInfo {
            pr_base_sha: base_ref_oid.to_string(),
            current_main_sha,
        }))
    } else {
        Ok(None)
    }
}

/// Run `gh pr merge <number> --repo <repo> --<method> [--delete-branch] [--auto]`
/// and return stdout on success or stderr on failure.
pub(crate) async fn run_gh_merge(
    pr_number: u64,
    repo: &str,
    merge_method: &str,
    delete_branch: bool,
    auto_merge: bool,
    token: &str,
) -> Result<String, String> {
    let pr_str = pr_number.to_string();
    let method_flag = format!("--{merge_method}");
    let mut args = vec!["pr", "merge", &pr_str, "--repo", repo, &method_flag];

    if delete_branch {
        args.push("--delete-branch");
    }
    if auto_merge {
        args.push("--auto");
    }

    run_gh_subprocess(&args, token).await
}

/// Spawn a `gh` subprocess with proper env scrubbing and token injection.
///
/// Returns stdout on success. On failure, returns an error string combining
/// exit code and stderr.
pub(crate) async fn run_gh_subprocess(args: &[&str], token: &str) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(args);
    cmd.env("GH_PROMPT_DISABLED", "1");

    // Scrub MIKA_* and GH_TOKEN, then re-inject the correct token.
    // Uses the same pattern as run_gh in builtin_handlers.rs.
    crate::skills::executor::scrub_mika_env_vars(&mut cmd);
    cmd.env("GH_TOKEN", token);

    cmd.kill_on_drop(true);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return Err("gh CLI not found — install from https://cli.github.com".to_string());
            }
            return Err(format!("Failed to spawn gh: {e}"));
        }
    };

    // Read stdout and stderr with bounded size to prevent memory exhaustion
    let stdout_handle = child.stdout.take().expect("stdout piped");
    let stderr_handle = child.stderr.take().expect("stderr piped");
    let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
    let mut stderr_buf = Vec::with_capacity(MAX_OUTPUT_LEN);

    let mut stdout_take = stdout_handle.take(MAX_OUTPUT_LEN as u64);
    let mut stderr_take = stderr_handle.take(MAX_OUTPUT_LEN as u64);
    let (stdout_res, stderr_res) = tokio::join!(
        stdout_take.read_to_end(&mut stdout_buf),
        stderr_take.read_to_end(&mut stderr_buf),
    );
    stdout_res.map_err(|e| format!("Failed to read stdout: {e}"))?;
    stderr_res.map_err(|e| format!("Failed to read stderr: {e}"))?;

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for gh: {e}"))?;

    let stdout = String::from_utf8_lossy(&stdout_buf);
    let stderr = String::from_utf8_lossy(&stderr_buf);

    if status.success() {
        Ok(stdout.into_owned())
    } else {
        let code_display = status
            .code()
            .map(|c| format!("exit code {c}"))
            .unwrap_or_else(|| "unknown exit code".to_string());

        let mut err = format!("gh {code_display}");
        if !stderr.is_empty() {
            err.push_str(": ");
            err.push_str(stderr.trim());
        } else if !stdout.is_empty() {
            err.push_str(": ");
            err.push_str(stdout.trim());
        }

        // Log for observability but return the full error to the agent
        warn!(
            args = ?args,
            exit_code = status.code(),
            stderr = %stderr.trim(),
            "gh subprocess failed"
        );

        Err(err)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Persist the supervisor's `$.claude_pilot.pr_url` after auto-merge is
/// enabled (mika#1211). Mirror of `dispatcher::try_extract_callback_metadata`
/// — resolves the supervisor via `ToolContext.callback_task_id → parent`,
/// gates on `trigger_type='manual' && source='self_dev'`, performs a
/// two-level shallow merge with existing metadata, and persists via
/// `update_task_metadata`. Fire-and-forget: errors are logged but never
/// propagated into the tool result.
///
/// Skip cases (silently): no callback context (conversation mode / mika-arch
/// dispatch), callback not found, callback has no parent, parent is not a
/// manual self_dev supervisor (milestone/project parent, operator task,
/// etc.).
async fn write_auto_merge_pr_url_to_supervisor(ctx: &ToolContext<'_>, pr_url: &str) {
    // 1. Conversation mode and other non-callback turns have no identifiable
    //    supervisor — skip without logging (expected, not anomalous).
    let callback_id = match ctx.callback_task_id {
        Some(id) => id,
        None => return,
    };

    // 2. Look up the callback task to find its parent.
    let callback = match ctx.db.get_task_unscoped(callback_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                callback_task_id = %callback_id,
                "pr_merge_with_gate: callback task not found for pr_url write"
            );
            return;
        }
        Err(e) => {
            warn!(
                callback_task_id = %callback_id,
                error = %e,
                "pr_merge_with_gate: failed to load callback task for pr_url write"
            );
            return;
        }
    };
    let parent_id = match callback.parent_task_id {
        Some(id) => id,
        None => return,
    };

    // 3. Verify parent is a manual self_dev supervisor — mirror the reaper's
    //    guard set (find_orphaned_parent_tasks WHERE trigger_type='manual'
    //    AND source='self_dev'). Skip milestone/project/operator parents.
    let parent = match ctx.db.get_task_unscoped(&parent_id).await {
        Ok(Some(t)) if t.trigger_type == "manual" && t.source.as_deref() == Some("self_dev") => t,
        Ok(_) => return,
        Err(e) => {
            warn!(
                parent_task_id = %parent_id,
                error = %e,
                "pr_merge_with_gate: failed to load supervisor task for pr_url write"
            );
            return;
        }
    };

    // 4. Build patch object and two-level shallow merge with existing metadata
    //    via the shared helper that update_task_status and
    //    try_extract_callback_metadata also use (see #489).
    let patch = serde_json::json!({"claude_pilot": {"pr_url": pr_url}});
    let merged = match &parent.metadata {
        Some(existing) => {
            if let Ok(mut base) = serde_json::from_str::<serde_json::Value>(existing) {
                crate::task_state::merge_metadata(&mut base, &patch);
                base
            } else {
                patch
            }
        }
        None => patch,
    };

    // 5. Persist via update_task_metadata (manual-only by SQL guard).
    persist_supervisor_metadata(ctx.db, &parent_id, &merged.to_string(), pr_url).await;
}

/// Tiny helper isolated for unit-test of the persist arm. Same three-arm
/// match shape as `try_extract_callback_metadata`.
async fn persist_supervisor_metadata(
    db: &AsyncDatabase,
    parent_id: &str,
    merged_json: &str,
    pr_url: &str,
) {
    match db.update_task_metadata(parent_id, merged_json).await {
        Ok(true) => info!(
            supervisor_task_id = %parent_id,
            pr_url = %pr_url,
            "pr_merge_with_gate: wrote pr_url to supervisor metadata on auto_merge_enabled"
        ),
        Ok(false) => warn!(
            supervisor_task_id = %parent_id,
            "pr_merge_with_gate: supervisor task not found for pr_url write"
        ),
        Err(e) => warn!(
            supervisor_task_id = %parent_id,
            error = %e,
            "pr_merge_with_gate: failed to persist pr_url to supervisor metadata"
        ),
    }
}

/// Try to extract an exit code integer from an error string like "gh exit code 1: ..."
fn parse_exit_code_from_error(err: &str) -> i32 {
    if let Some(rest) = err.strip_prefix("gh exit code ")
        && let Some(code_str) = rest.split(':').next()
        && let Ok(code) = code_str.trim().parse::<i32>()
    {
        return code;
    }
    -1
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    // -- classify_checks tests --

    #[test]
    fn classify_all_passed() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Lint".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_with_skipped() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Optional".to_string(),
                state: "SKIPPED".to_string(),
                bucket: "skipping".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_empty_checks() {
        assert_eq!(classify_checks(&[]), CheckClassification::AllPassed);
    }

    #[test]
    fn classify_has_pending() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
            GhCheck {
                name: "Build".to_string(),
                state: "IN_PROGRESS".to_string(),
                bucket: "pending".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::HasPending);
    }

    #[test]
    fn classify_has_failures() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                bucket: "fail".to_string(),
                link: Some("https://github.com/org/repo/actions/runs/123".to_string()),
            },
            GhCheck {
                name: "Lint".to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            },
        ];
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    #[test]
    fn classify_cancelled_is_failure() {
        let checks = vec![GhCheck {
            name: "CI".to_string(),
            state: "CANCELLED".to_string(),
            bucket: "cancel".to_string(),
            link: None,
        }];
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    #[test]
    fn classify_mixed_failure_and_pending_is_failure() {
        let checks = vec![
            GhCheck {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                bucket: "fail".to_string(),
                link: None,
            },
            GhCheck {
                name: "Build".to_string(),
                state: "PENDING".to_string(),
                bucket: "pending".to_string(),
                link: None,
            },
        ];
        // Failures take priority over pending
        assert_eq!(classify_checks(&checks), CheckClassification::HasFailures);
    }

    // -- validate_repo tests --

    #[test]
    fn validate_repo_valid() {
        assert!(validate_repo("senara-solutions/mika").is_ok());
        assert!(validate_repo("owner/repo").is_ok());
        assert!(validate_repo("my-org/my.repo").is_ok());
        assert!(validate_repo("org_name/repo_name").is_ok());
    }

    #[test]
    fn validate_repo_invalid() {
        assert!(validate_repo("").is_err());
        assert!(validate_repo("noslash").is_err());
        assert!(validate_repo("too/many/slashes").is_err());
        assert!(validate_repo("has spaces/repo").is_err());
        assert!(validate_repo("owner/").is_err());
        assert!(validate_repo("/repo").is_err());
        assert!(validate_repo("https://github.com/owner/repo").is_err());
        assert!(validate_repo("owner/repo --flag").is_err());
    }

    #[test]
    fn validate_repo_too_long() {
        let long_repo = format!("{}/{}", "a".repeat(100), "b".repeat(101));
        assert!(validate_repo(&long_repo).is_err());
    }

    // -- validate_merge_method tests --

    #[test]
    fn validate_merge_method_valid() {
        assert!(validate_merge_method("squash").is_ok());
        assert!(validate_merge_method("merge").is_ok());
        assert!(validate_merge_method("rebase").is_ok());
    }

    #[test]
    fn validate_merge_method_invalid() {
        assert!(validate_merge_method("delete").is_err());
        assert!(validate_merge_method("squash --title evil").is_err());
        assert!(validate_merge_method("").is_err());
    }

    // -- GhCheck deserialization tests --

    #[test]
    fn deserialize_gh_checks_output() {
        let json = r#"[
            {
                "name": "CI / test",
                "state": "SUCCESS",
                "bucket": "pass",
                "link": "https://github.com/org/repo/actions/runs/123"
            },
            {
                "name": "Pipeline Artifacts",
                "state": "FAILURE",
                "bucket": "fail",
                "link": "https://github.com/org/repo/actions/runs/456"
            }
        ]"#;

        let checks: Vec<GhCheck> = serde_json::from_str(json).unwrap();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, "CI / test");
        assert_eq!(checks[0].bucket, "pass");
        assert_eq!(checks[1].name, "Pipeline Artifacts");
        assert_eq!(checks[1].bucket, "fail");
    }

    #[test]
    fn deserialize_gh_checks_no_link() {
        let json = r#"[{"name": "test", "state": "PENDING", "bucket": "pending"}]"#;
        let checks: Vec<GhCheck> = serde_json::from_str(json).unwrap();
        assert_eq!(checks.len(), 1);
        assert!(checks[0].link.is_none());
    }

    // -- MergeGateResult serialization tests --

    #[test]
    fn serialize_merged_result() {
        let result = MergeGateResult::Merged;
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "merged");
    }

    #[test]
    fn serialize_blocked_result_required_check_failed() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::RequiredCheckFailed {
                failing_checks: vec![CheckInfo {
                    name: "CI".to_string(),
                    state: "FAILURE".to_string(),
                    link: Some("https://example.com".to_string()),
                }],
            },
            failing_checks: vec![CheckInfo {
                name: "CI".to_string(),
                state: "FAILURE".to_string(),
                link: Some("https://example.com".to_string()),
            }],
            detail: "1 required check(s) failed".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        // Backward compat: top-level failing_checks
        assert_eq!(json["failing_checks"][0]["name"], "CI");
        assert_eq!(json["failing_checks"][0]["link"], "https://example.com");
        // New: reason field with tagged union
        assert_eq!(json["reason"]["reason"], "required_check_failed");
        assert_eq!(json["reason"]["failing_checks"][0]["name"], "CI");
        assert_eq!(json["detail"], "1 required check(s) failed");
    }

    #[test]
    fn serialize_blocked_merge_conflict() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::MergeConflict,
            failing_checks: vec![],
            detail: "PR has merge conflicts — rebase needed".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "merge_conflict");
        assert_eq!(json["failing_checks"].as_array().unwrap().len(), 0);
        assert_eq!(json["detail"], "PR has merge conflicts — rebase needed");
    }

    #[test]
    fn serialize_blocked_missing_approval() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::MissingApproval,
            failing_checks: vec![],
            detail: "Required reviews not met".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "missing_approval");
    }

    #[test]
    fn serialize_blocked_draft() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::Draft,
            failing_checks: vec![],
            detail: "PR is a draft".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "draft");
    }

    #[test]
    fn serialize_blocked_pr_closed() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::PrClosed,
            failing_checks: vec![],
            detail: "PR is closed".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "pr_closed");
    }

    #[test]
    fn serialize_gate_error() {
        let result = MergeGateResult::GateError {
            kind: GateErrorKind::GhCliFailure { exit_code: 1 },
            detail: "gh exit code 1: no checks reported".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert_eq!(json["kind"]["kind"], "gh_cli_failure");
        assert_eq!(json["kind"]["exit_code"], 1);
        assert_eq!(json["detail"], "gh exit code 1: no checks reported");
    }

    #[test]
    fn serialize_gate_error_network() {
        let result = MergeGateResult::GateError {
            kind: GateErrorKind::NetworkError,
            detail: "Connection refused".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert_eq!(json["kind"]["kind"], "network_error");
    }

    #[test]
    fn serialize_gate_error_parse() {
        let result = MergeGateResult::GateError {
            kind: GateErrorKind::ParseError,
            detail: "Invalid JSON from gh".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert_eq!(json["kind"]["kind"], "parse_error");
    }

    #[test]
    fn serialize_gate_error_unknown() {
        let result = MergeGateResult::GateError {
            kind: GateErrorKind::Unknown,
            detail: "Merge failed: unexpected output".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert_eq!(json["kind"]["kind"], "unknown");
        assert_eq!(json["detail"], "Merge failed: unexpected output");
    }

    #[test]
    fn serialize_auto_merge_result() {
        let result = MergeGateResult::AutoMergeEnabled {
            pending_checks: vec![CheckInfo {
                name: "Build".to_string(),
                state: "IN_PROGRESS".to_string(),
                link: None,
            }],
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "auto_merge_enabled");
        assert_eq!(json["pending_checks"][0]["name"], "Build");
        // link should be absent (skip_serializing_if = None)
        assert!(json["pending_checks"][0].get("link").is_none());
    }

    #[test]
    fn serialize_already_merged_result() {
        let result = MergeGateResult::AlreadyMerged;
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "already_merged");
    }

    // -- Preflight classification tests --

    #[test]
    fn preflight_conflicting_returns_merge_conflict() {
        let preflight = PrPreflight {
            mergeable: "CONFLICTING".to_string(),
            merge_state_status: "DIRTY".to_string(),
            is_draft: false,
            state: "OPEN".to_string(),
            base_ref_oid: String::new(),
        };
        let result = classify_preflight(&preflight);
        assert_eq!(
            result,
            Some(MergeGateResult::Blocked {
                reason: BlockReason::MergeConflict,
                failing_checks: vec![],
                detail: "PR has merge conflicts — rebase needed".to_string(),
            })
        );
    }

    #[test]
    fn preflight_dirty_returns_merge_conflict() {
        let preflight = PrPreflight {
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: "DIRTY".to_string(),
            is_draft: false,
            state: "OPEN".to_string(),
            base_ref_oid: String::new(),
        };
        let result = classify_preflight(&preflight);
        assert_eq!(
            result,
            Some(MergeGateResult::Blocked {
                reason: BlockReason::MergeConflict,
                failing_checks: vec![],
                detail: "PR has merge conflicts — rebase needed".to_string(),
            })
        );
    }

    #[test]
    fn preflight_mergeable_open_passes_through() {
        let preflight = PrPreflight {
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            is_draft: false,
            state: "OPEN".to_string(),
            base_ref_oid: String::new(),
        };
        assert_eq!(classify_preflight(&preflight), None);
    }

    #[test]
    fn preflight_closed_returns_pr_closed() {
        let preflight = PrPreflight {
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            is_draft: false,
            state: "CLOSED".to_string(),
            base_ref_oid: String::new(),
        };
        let result = classify_preflight(&preflight);
        assert_eq!(
            result,
            Some(MergeGateResult::Blocked {
                reason: BlockReason::PrClosed,
                failing_checks: vec![],
                detail: "PR is closed".to_string(),
            })
        );
    }

    #[test]
    fn preflight_merged_returns_already_merged() {
        let preflight = PrPreflight {
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            is_draft: false,
            state: "MERGED".to_string(),
            base_ref_oid: String::new(),
        };
        assert_eq!(
            classify_preflight(&preflight),
            Some(MergeGateResult::AlreadyMerged)
        );
    }

    #[test]
    fn preflight_draft_returns_draft() {
        let preflight = PrPreflight {
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            is_draft: true,
            state: "OPEN".to_string(),
            base_ref_oid: String::new(),
        };
        let result = classify_preflight(&preflight);
        assert_eq!(
            result,
            Some(MergeGateResult::Blocked {
                reason: BlockReason::Draft,
                failing_checks: vec![],
                detail: "PR is a draft — convert to ready before merging".to_string(),
            })
        );
    }

    /// #792 regression test (Unit 1, tool-layer):
    /// Mock gh pr view returning CONFLICTING + DIRTY + empty statusCheckRollup.
    /// Assert the tool returns Blocked { reason: MergeConflict } WITHOUT
    /// attempting any gh pr merge call.
    #[test]
    fn regression_792_conflicting_pr_returns_blocked_merge_conflict() {
        // This is the exact state from PR #792 that triggered the incident:
        // mergeable: CONFLICTING, mergeStateStatus: DIRTY, statusCheckRollup: []
        let preflight = PrPreflight {
            mergeable: "CONFLICTING".to_string(),
            merge_state_status: "DIRTY".to_string(),
            is_draft: false,
            state: "OPEN".to_string(),
            base_ref_oid: String::new(),
        };

        let result = classify_preflight(&preflight);

        // Must return Blocked { MergeConflict }, NOT an error string
        match result {
            Some(MergeGateResult::Blocked {
                reason: BlockReason::MergeConflict,
                ..
            }) => {} // Expected
            other => panic!(
                "Expected Blocked {{ MergeConflict }}, got: {other:?}. \
                 The tool must detect CONFLICTING state before attempting merge."
            ),
        }
    }

    // -- Backward compatibility test --

    #[test]
    fn backward_compat_blocked_has_action_and_failing_checks_at_top_level() {
        // Ensure existing prompts that branch on action == "blocked"
        // and read failing_checks at the top level still work.
        let result = MergeGateResult::Blocked {
            reason: BlockReason::RequiredCheckFailed {
                failing_checks: vec![CheckInfo {
                    name: "CI / test".to_string(),
                    state: "FAILURE".to_string(),
                    link: None,
                }],
            },
            failing_checks: vec![CheckInfo {
                name: "CI / test".to_string(),
                state: "FAILURE".to_string(),
                link: None,
            }],
            detail: "1 required check(s) failed".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();

        // These are the fields existing prompts rely on:
        assert_eq!(json["action"], "blocked");
        assert!(json["failing_checks"].is_array());
        assert_eq!(json["failing_checks"][0]["name"], "CI / test");
    }

    // -- parse_exit_code_from_error tests --

    #[test]
    fn parse_exit_code_success() {
        assert_eq!(
            parse_exit_code_from_error("gh exit code 1: no checks reported"),
            1
        );
        assert_eq!(
            parse_exit_code_from_error("gh exit code 128: not a git repository"),
            128
        );
    }

    #[test]
    fn parse_exit_code_fallback() {
        assert_eq!(parse_exit_code_from_error("some random error"), -1);
        assert_eq!(parse_exit_code_from_error(""), -1);
    }

    // -- Tool integration tests --

    #[tokio::test]
    async fn test_missing_github_token() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // ctx.github_token is None by default in TestHarness

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "senara-solutions/mika"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("GitHub token required"));
    }

    #[tokio::test]
    async fn test_invalid_repo_format() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "not-a-valid-repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid repo format"));
    }

    #[tokio::test]
    async fn test_invalid_merge_method() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 42,
            "repo": "owner/repo",
            "merge_method": "delete-everything"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid merge_method"));
    }

    #[tokio::test]
    async fn test_missing_pr_number() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "repo": "owner/repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("pr_number"));
    }

    #[tokio::test]
    async fn test_zero_pr_number() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = PrMergeWithGateTool;
        let input = json!({
            "pr_number": 0,
            "repo": "owner/repo"
        });

        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("pr_number"));
    }

    #[test]
    fn test_tool_definition() {
        let tool = PrMergeWithGateTool;
        assert_eq!(tool.name(), "pr_merge_with_gate");
        assert_eq!(tool.timeout_secs(), Some(60));

        let def = tool.definition();
        assert_eq!(def.name, "pr_merge_with_gate");
        assert!(def.description.contains("CI gate"));
        assert!(def.description.contains("auto_merge_enabled"));
        assert!(def.description.contains("gate_errored"));

        // Verify schema has required fields
        let props = &def.input_schema["properties"];
        assert!(props.get("pr_number").is_some());
        assert!(props.get("repo").is_some());
        assert!(props.get("merge_method").is_some());
        assert!(props.get("delete_branch").is_some());

        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("pr_number")));
        assert!(required.contains(&json!("repo")));
    }

    // -- mika#1211: supervisor pr_url write tests --
    //
    // These tests exercise `write_auto_merge_pr_url_to_supervisor` directly
    // instead of `tool.execute(...)` — the full execute path requires `gh`
    // CLI subprocess calls. The helper is the entire surface this fix adds,
    // so direct unit coverage is sufficient.

    use crate::async_db::AsyncDatabase;
    use crate::db::{Database, NewTask};
    use crate::tools::ToolContext;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU32};

    /// Seed a manual self_dev supervisor task and a callback child for it.
    /// Returns (supervisor_id, callback_id).
    async fn seed_supervisor_and_callback(db: &AsyncDatabase) -> (String, String) {
        seed_supervisor_and_callback_with(db, Some("self_dev"), "manual", None).await
    }

    async fn seed_supervisor_and_callback_with(
        db: &AsyncDatabase,
        source: Option<&str>,
        trigger: &str,
        initial_metadata: Option<&str>,
    ) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement mika#1211".to_string(),
            trigger_type: trigger.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: source.map(String::from),
            metadata: initial_metadata.map(String::from),
            r#type: None,
            dispatch_class: None,
        };
        let supervisor_id = db.create_task(parent).await.unwrap();

        let callback = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(supervisor_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let callback_id = db.create_task(callback).await.unwrap();
        (supervisor_id, callback_id)
    }

    fn make_ctx<'a>(
        db: &'a AsyncDatabase,
        counter: &'a AtomicU32,
        skills_dirty: &'a AtomicBool,
        pr_review_posted: &'a AtomicBool,
        tool_arg_suffix_rejected: &'a AtomicBool,
        callback_task_id: Option<&'a str>,
    ) -> ToolContext<'a> {
        ToolContext {
            db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: Path::new("/tmp/mika-test"),
            global_home_dir: None,
            core_memory_edit_count: counter,
            is_onboarding: false,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: callback_task_id.is_some(),
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected,
            scope_task_id: None,
        }
    }

    async fn read_pr_url(db: &AsyncDatabase, supervisor_id: &str) -> Option<String> {
        let parent = db.get_task_unscoped(supervisor_id).await.unwrap().unwrap();
        parent.metadata.and_then(|m| {
            let v: serde_json::Value = serde_json::from_str(&m).ok()?;
            v.get("claude_pilot")?
                .get("pr_url")?
                .as_str()
                .map(String::from)
        })
    }

    #[tokio::test]
    async fn write_pr_url_writes_to_supervisor_when_callback_context_present() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        let (sup_id, cb_id) = seed_supervisor_and_callback(&db).await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        let pr_url = "https://github.com/senara-solutions/mika/pull/1206";
        write_auto_merge_pr_url_to_supervisor(&ctx, pr_url).await;

        assert_eq!(read_pr_url(&db, &sup_id).await.as_deref(), Some(pr_url));
    }

    #[tokio::test]
    async fn write_pr_url_skips_when_no_callback_context() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        let (sup_id, _cb_id) = seed_supervisor_and_callback(&db).await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, None);

        write_auto_merge_pr_url_to_supervisor(
            &ctx,
            "https://github.com/senara-solutions/mika/pull/1206",
        )
        .await;

        assert!(read_pr_url(&db, &sup_id).await.is_none());
    }

    #[tokio::test]
    async fn write_pr_url_skips_when_supervisor_source_not_self_dev() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        // Operator-sourced parent (not self_dev) — must be skipped.
        let (sup_id, cb_id) =
            seed_supervisor_and_callback_with(&db, Some("operator"), "manual", None).await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        write_auto_merge_pr_url_to_supervisor(
            &ctx,
            "https://github.com/senara-solutions/mika/pull/9",
        )
        .await;

        assert!(read_pr_url(&db, &sup_id).await.is_none());
    }

    #[tokio::test]
    async fn write_pr_url_skips_when_supervisor_trigger_not_manual() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        // Callback-typed parent (chained callback) — must be skipped.
        let (sup_id, cb_id) =
            seed_supervisor_and_callback_with(&db, Some("self_dev"), "callback", None).await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        write_auto_merge_pr_url_to_supervisor(
            &ctx,
            "https://github.com/senara-solutions/mika/pull/9",
        )
        .await;

        assert!(read_pr_url(&db, &sup_id).await.is_none());
    }

    #[tokio::test]
    async fn write_pr_url_preserves_existing_claude_pilot_fields() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        let initial = r#"{"claude_pilot":{"session_id":"abc","cost_usd":"1.50","turns":42}}"#;
        let (sup_id, cb_id) =
            seed_supervisor_and_callback_with(&db, Some("self_dev"), "manual", Some(initial)).await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        let pr_url = "https://github.com/senara-solutions/mika/pull/1206";
        write_auto_merge_pr_url_to_supervisor(&ctx, pr_url).await;

        let parent = db.get_task_unscoped(&sup_id).await.unwrap().unwrap();
        let metadata: serde_json::Value =
            serde_json::from_str(parent.metadata.as_ref().unwrap()).unwrap();
        let cp = &metadata["claude_pilot"];
        assert_eq!(cp["session_id"], "abc");
        assert_eq!(cp["cost_usd"], "1.50");
        assert_eq!(cp["turns"], 42);
        assert_eq!(cp["pr_url"], pr_url);
    }

    #[tokio::test]
    async fn write_pr_url_is_idempotent() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        let pr_url = "https://github.com/senara-solutions/mika/pull/1206";
        let initial = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
        let (sup_id, cb_id) =
            seed_supervisor_and_callback_with(&db, Some("self_dev"), "manual", Some(&initial))
                .await;

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        write_auto_merge_pr_url_to_supervisor(&ctx, pr_url).await;
        write_auto_merge_pr_url_to_supervisor(&ctx, pr_url).await;

        assert_eq!(read_pr_url(&db, &sup_id).await.as_deref(), Some(pr_url));
    }

    #[tokio::test]
    async fn write_pr_url_skips_when_callback_has_no_parent() {
        let db = AsyncDatabase::new({
            let d = Database::open_in_memory().unwrap();
            d.create_session("test-session", "mika", "cli").unwrap();
            d
        });
        // Orphan callback — no parent_task_id.
        let orphan = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let cb_id = db.create_task(orphan).await.unwrap();

        let counter = AtomicU32::new(0);
        let sd = AtomicBool::new(false);
        let pr = AtomicBool::new(false);
        let tas = AtomicBool::new(false);
        let ctx = make_ctx(&db, &counter, &sd, &pr, &tas, Some(&cb_id));

        // Should not panic or error — just returns early.
        write_auto_merge_pr_url_to_supervisor(
            &ctx,
            "https://github.com/senara-solutions/mika/pull/9",
        )
        .await;
    }

    // -- Behind-main assertion tests (#1577) --

    #[test]
    fn behind_main_block_reason_serializes_correctly() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::BehindMain {
                pr_base_sha: "abc123".to_string(),
                current_main_sha: "def456".to_string(),
            },
            failing_checks: vec![],
            detail: "PR is behind main".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "behind_main");
        assert_eq!(json["reason"]["pr_base_sha"], "abc123");
        assert_eq!(json["reason"]["current_main_sha"], "def456");
    }

    #[test]
    fn preflight_deserializes_base_ref_oid() {
        let json = r#"{
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "isDraft": false,
            "state": "OPEN",
            "baseRefOid": "abc123def456"
        }"#;
        let preflight: PrPreflight = serde_json::from_str(json).unwrap();
        assert_eq!(preflight.base_ref_oid, "abc123def456");
    }

    #[test]
    fn preflight_base_ref_oid_defaults_to_empty() {
        // Pre-existing gh output without baseRefOid should still deserialize
        let json = r#"{
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "isDraft": false,
            "state": "OPEN"
        }"#;
        let preflight: PrPreflight = serde_json::from_str(json).unwrap();
        assert_eq!(preflight.base_ref_oid, "");
    }
}
