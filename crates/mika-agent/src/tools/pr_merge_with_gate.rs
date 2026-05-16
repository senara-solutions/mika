use std::sync::LazyLock;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tracing::warn;

use super::{Tool, ToolContext, ToolOutput};

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
        "mergeable,mergeStateStatus,isDraft,state",
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
}
