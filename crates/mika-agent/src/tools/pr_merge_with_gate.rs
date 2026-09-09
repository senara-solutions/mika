use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
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
                IMPORTANT: 'branch_updated' means the PR was behind main and GitHub accepted an \
                update of its branch. No merge was attempted. Do NOT call this tool again for \
                this PR in the same turn and do NOT rebase by hand — the update moves the head \
                to a new commit with no CI result. The PR then needs a FRESH QA review: moving \
                the head SHA invalidates the approval that pointed at the old one. End the \
                turn and say that the behind-main state is repaired but the review is not.\n\n\
                After a successful merge (action: 'merged'), update the task status before \
                reporting to the user.\n\n\
                Returns a structured JSON response with an 'action' field. Possible actions: \
                'merged', 'auto_merge_enabled', 'blocked', 'already_merged', 'gate_errored', \
                'branch_updated'. Branch on 'action' to determine next steps."
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
                // A credential-scope 403 surfaces here first when the App/PAT
                // cannot even read the private repo (mika#1616).
                let result = classify_credential_scope_error(&e.message, repo).unwrap_or(
                    MergeGateResult::GateError {
                        kind: GateErrorKind::GhCliFailure {
                            exit_code: e.exit_code.unwrap_or(-1),
                        },
                        detail: e.message,
                    },
                );
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
            }
        };

        // Classify preflight result
        if let Some(result) = classify_preflight(&preflight) {
            return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
        }

        // -- Step 1c: Forge-gate perimeter check (mika#1829) --
        //
        // Fetch touched files from GitHub, classify via perimeter rules.
        // If the PR touches any DECISION-CORE zone, block with
        // `HumanGateRequired`. Fail-closed on gh-CLI fetch error
        // (better to hold a mechanical PR than to auto-merge a
        // decision-core PR).
        //
        // Non-negotiable (b): diff is authority. This step is
        // structurally independent of PR labels / body prose.
        let perimeter_verdict = match crate::perimeter::fetch::fetch_pr_files(
            pr_number, repo, token,
        )
        .await
        {
            Ok(files) => crate::perimeter::classify_pr_files(&files),
            Err(e) => {
                warn!(
                    error = %e,
                    pr_number,
                    repo,
                    "pr_merge_with_gate: perimeter file fetch failed — fail-closed to DECISION-CORE (mika#1829)"
                );
                crate::perimeter::PrClassification {
                    verdict: crate::perimeter::Classification::DecisionCore,
                    mechanical_files: Vec::new(),
                    decision_core_files: vec![format!("<fetch-error: {e}>")],
                }
            }
        };
        if perimeter_verdict.verdict == crate::perimeter::Classification::DecisionCore {
            let detail = format!(
                "Forge-gate: PR touches DECISION-CORE zone(s) — operator must merge manually. {}",
                perimeter_verdict.summary()
            );
            let result = MergeGateResult::Blocked {
                reason: BlockReason::HumanGateRequired {
                    decision_core_files: perimeter_verdict.decision_core_files.clone(),
                    summary: perimeter_verdict.summary(),
                },
                failing_checks: vec![],
                detail,
            };
            return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
        }

        // -- Step 2: Fetch required check statuses --
        let checks_result = run_gh_checks(pr_number, repo, token).await;
        let checks = match checks_result {
            Ok(c) => c,
            Err(e) => {
                let result = classify_credential_scope_error(&e, repo).unwrap_or(
                    MergeGateResult::GateError {
                        kind: GateErrorKind::GhCliFailure {
                            exit_code: parse_exit_code_from_error(&e),
                        },
                        detail: format!("Failed to fetch check statuses: {e}"),
                    },
                );
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
            }
        };

        // -- Step 3: Classify and act --
        let classification = classify_checks(&checks);

        // -- Step 3a: A red PR is reported as red, before anything else --
        // This arm runs ahead of the behind-main step below so a PR that is both
        // behind and failing reports the failing checks, not "branch updated".
        if classification == CheckClassification::HasFailures {
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
                failing_checks: failing.clone(),
                detail: format!("{} required check(s) failed", failing.len()),
            };
            return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
        }

        // -- Step 3b: Behind-main assertion (#1577) + remediation (mika#2238) --
        //
        // Placed AFTER the perimeter gate and the CI-failure arm, and BEFORE the
        // auto-merge arm. That last part is load-bearing: `--auto` on a PR that
        // is behind would let GitHub merge it behind our backs once the pending
        // checks go green, which is the #1577 defect. The behind state has to be
        // settled before auto-merge is armed.
        //
        // Fail-open on the DETECTION API error, as before; the remediation
        // itself never fails open (see `disposition_for_remediation`).
        match is_behind_main(&preflight.base_ref_oid, repo, token).await {
            Ok(Some(info)) => {
                let remediation = remediate_behind_main(
                    "pr_merge_with_gate",
                    pr_number,
                    repo,
                    &preflight.base_ref_name,
                    token,
                    &info,
                )
                .await;
                if let Some(result) = disposition_for_remediation(&remediation, repo, &info) {
                    return Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?));
                }
                // `None` — the PR turned out not to be behind. Continue the gate.
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

        match classification {
            // Handled above by step 3a, which returns.
            CheckClassification::HasFailures => unreachable!("HasFailures returns at step 3a"),
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
                        let result = classify_credential_scope_error(&e, repo).unwrap_or(
                            MergeGateResult::GateError {
                                kind: GateErrorKind::GhCliFailure {
                                    exit_code: parse_exit_code_from_error(&e),
                                },
                                detail: format!("Auto-merge failed: {e}"),
                            },
                        );
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
                } else if let Some(result) = classify_credential_scope_error(&output, repo) {
                    // Credential-scope 403 takes priority over the generic
                    // draft/conflict/review classification below (mika#1616).
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
    /// The PR was behind `main` and GitHub accepted an update of its branch
    /// (mika#2238). Deliberately NOT a `Blocked` variant: a behind-main state
    /// that was repaired is not a blockage, and collapsing the two would leave
    /// the agent unable to tell "the mechanical state is fixed" from "something
    /// is wrong". No merge was attempted and none must be attempted this turn —
    /// the new head commit has no CI result yet.
    ///
    /// **This does not, on its own, lead to a merge.** Moving the head SHA also
    /// invalidates the QA approval that pointed at the old one, so the stale-SHA
    /// gate in `ci_success_handler` holds the PR until QA re-reviews. This
    /// variant means "the behind-main state is repaired", never "the merge will
    /// now happen by itself". Teaching that gate to follow an update-branch
    /// merge is a change to a review gate and is tracked separately.
    #[serde(rename = "branch_updated")]
    BranchUpdated {
        pr_base_sha: String,
        new_main_sha: String,
    },
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
    /// PR touches DECISION-CORE zone(s) (forge-gate perimeter, mika#1829).
    /// Operator must merge manually. The `decision_core_files` field lists the
    /// concrete paths that tripped the gate.
    #[serde(rename = "human_gate_required")]
    HumanGateRequired {
        decision_core_files: Vec<String>,
        summary: String,
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
    /// The merge credential (GitHub App installation token or PAT) lacks write
    /// access to `repo` — surfaces from `gh` as a 403 / "Resource not
    /// accessible by integration" / forbidden response (mika#1616). Distinct
    /// from `GhCliFailure` so the agent reports a concrete, actionable cause
    /// (install the App / widen the PAT scope) instead of paraphrasing an
    /// opaque exit code into a fabricated guess.
    #[serde(rename = "credential_scope")]
    CredentialScope { repo: String },
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
    /// The NAME of the base branch (`main`, a stacking parent, a release
    /// branch). `is_behind_main` compares `base_ref_oid` against
    /// `refs/heads/main` unconditionally, so for a PR based on anything else
    /// the comparison always reports "behind" (mika#2238). Reading it as a fact
    /// was merely noisy while the gate only declined; it stopped being harmless
    /// once the gate started pushing a merge commit in response.
    #[serde(default)]
    pub(crate) base_ref_name: String,
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
        "mergeable,mergeStateStatus,isDraft,state,baseRefOid,baseRefName",
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

// ---------------------------------------------------------------------------
// Behind-main remediation (mika#2238)
// ---------------------------------------------------------------------------
//
// `is_behind_main` above DETECTS the stale state; nothing repaired it. The
// only remediation shipped with #1577 was a sentence addressed to an LLM
// ("Rebase the PR onto main before merging") naming no tool — and every merge
// onto `main` makes every other open PR behind, so this was not an edge case
// but the default state as soon as a second PR exists. Measured on mika#2236:
// APPROVED + CI-green + mergeable at 08:25:08Z, closed by a human at 10:00:36Z.
//
// The sequence below is a RENDEZVOUS, not a retry loop. `update-branch` creates
// a NEW commit on the PR head, so the green `statusCheckRollup` the gate just
// read belongs to the PREVIOUS commit. Merging straight after the update would
// put a commit no CI validated onto `main` — precisely the failure #1577 was
// written to close. So the turn ENDS after the update, and GitHub's fresh
// `check_suite success` webhook re-enters `ci_success_handler`, which is
// already the handler for that event. No new waiting mechanism is introduced.

/// Soft capacity of the update-branch attempt ledger.
const UPDATE_ATTEMPT_CAP: usize = 256;

/// Entries older than this are dropped on the next capacity-triggered sweep.
///
/// This is a memory bound, not a retry policy. It is set far above any CI
/// cycle so it cannot re-arm a thrash loop: a PR still behind the *same* main
/// HEAD six hours later is a stalled PR, not a PR being hammered. A process
/// restart re-arms the same way, and for the same reason is harmless.
const UPDATE_ATTEMPT_TTL: Duration = Duration::from_secs(6 * 3600);

/// Process-global ledger of update-branch attempts.
/// Key = `"{repo}#{pr}@{target_main_sha}"`, value = monotonic `Instant`.
static UPDATE_ATTEMPTS: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);

/// Outcome of one `update-branch` call against a PR.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UpdateBranchOutcome {
    /// GitHub accepted the update — a new commit now sits on the PR head.
    Updated,
    /// GitHub declined because the branch is not behind after all (benign race).
    AlreadyUpToDate,
    /// The update revealed a real content conflict — resolution is required.
    Conflict(String),
    /// Permission, network, malformed response, or `gh` itself missing.
    Failed(String),
}

/// What the behind-main path decided for this PR, in this turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BehindMainRemediation {
    /// The branch was brought up to date. A fresh CI run is expected.
    Updated,
    /// The anti-thrash guard already spent this PR's attempt at this main SHA.
    AlreadyAttempted,
    /// The PR turned out not to be behind after all — the gate may continue.
    NotBehind,
    /// The update hit a real conflict.
    Conflict(String),
    /// The update did not go through (permission, API, `gh` missing). The string
    /// is the RAW `gh` error, which is why this is the only variant routed
    /// through `classify_credential_scope_error`.
    Failed(String),
    /// GitHub reported the branch was already up to date and the re-read
    /// disagreed, or the re-read itself failed.
    ///
    /// Separate from [`BehindMainRemediation::Failed`] for two reasons, both
    /// load-bearing. It is a different fact — the two sources of truth
    /// contradict each other, rather than an operation failing — so it earns
    /// its own `outcome` in the trace. And its string is composed prose that
    /// embeds SHAs, which must never reach `classify_credential_scope_error`:
    /// that helper matches the bare substring `403`, and a 40-hex SHA contains
    /// `403` about 1% of the time, which would turn a state contradiction into
    /// a confident, wrong "install the GitHub App" instruction.
    Contradiction(String),
    /// The PR's base branch is not `main`, so the behind-main comparison — which
    /// resolves `refs/heads/main` unconditionally (#1577) — does not describe
    /// this PR. Detection was a false positive; repairing it would push a real
    /// merge commit onto the head every time `main` moved.
    BaseNotMain(String),
}

/// Claim the single update-branch attempt allowed for `(pr, target_main_sha)`.
///
/// Returns `true` for the caller that may proceed, `false` for every later one.
/// The key is the SHA of `main` being aimed at rather than an attempt counter
/// (KTD-2): it is idempotent by construction and needs no reset. When another
/// PR merges, the target SHA changes, which legitimately re-opens one attempt —
/// so under a sustained merge stream a PR can stay behind indefinitely without
/// ever looping. That is deliberate: the starvation stays visible in the trace
/// rather than being hidden by an abandonment.
pub(crate) fn claim_update_branch_attempt(
    pr_number: u64,
    repo: &str,
    target_main_sha: &str,
) -> bool {
    claim_update_attempt_in(
        &UPDATE_ATTEMPTS,
        pr_number,
        repo,
        target_main_sha,
        Instant::now(),
    )
}

/// Give back a claim whose attempt demonstrably did not happen.
///
/// The claim is spent optimistically, before the call — it has to be, or two
/// concurrent webhooks both see it free and both fire an update. But an attempt
/// that failed to reach GitHub produced no commit, so it is not the thrash the
/// cap exists to stop, and keeping it spent would strand the PR: nothing evicts
/// the entry until the ledger hits its soft cap, and with a serialized dispatch
/// `main` may not advance for hours. A permanent stall is exactly the state
/// mika#2238 exists to end, so the guard must not manufacture one.
///
/// Released only for `Failed` — a `Conflict` is a stable fact about the PR that
/// re-asking cannot change, and an `Updated` is the case the cap is for.
///
/// The cost of releasing, stated: a persistently failing update (a 403) is
/// retried once per webhook rather than once per `main` SHA. That is a bounded
/// number of extra `gh` calls on a PR that is already blocked and already
/// notifying the operator — cheaper than a PR nobody comes back to.
fn release_update_branch_attempt(pr_number: u64, repo: &str, target_main_sha: &str) {
    UPDATE_ATTEMPTS.remove(&format!("{repo}#{pr_number}@{target_main_sha}"));
}

/// Core of [`claim_update_branch_attempt`], parameterized over the backing map
/// and the notion of "now" so it can be exercised without the process-global
/// ledger or real sleeps. Mirrors [`crate::server::check_suite_dedup`].
fn claim_update_attempt_in(
    map: &DashMap<String, Instant>,
    pr_number: u64,
    repo: &str,
    target_main_sha: &str,
    now: Instant,
) -> bool {
    let key = format!("{repo}#{pr_number}@{target_main_sha}");

    // Amortized eviction, done BEFORE taking the per-key shard lock — `retain`
    // locks every shard, so running it while holding an entry lock deadlocks.
    if map.len() >= UPDATE_ATTEMPT_CAP {
        map.retain(|_, &mut stored| now.saturating_duration_since(stored) < UPDATE_ATTEMPT_TTL);
    }

    // Atomic check-and-insert: the shard lock is held across read and write, so
    // a concurrent burst on one key yields exactly one `true`.
    match map.entry(key) {
        Entry::Occupied(mut e) => {
            if now.saturating_duration_since(*e.get()) < UPDATE_ATTEMPT_TTL {
                false
            } else {
                e.insert(now);
                true
            }
        }
        Entry::Vacant(e) => {
            e.insert(now);
            true
        }
    }
}

/// Ask GitHub to bring the PR's head branch up to date with its base.
///
/// **This is the only call site of update-branch in the codebase (R1).** A
/// source-scan test pins that; a behavioural test cannot, because a second
/// caller would not make any assertion fail — it would only make the
/// anti-thrash guard bypassable.
///
/// Goes through the REST endpoint rather than `gh pr update-branch`, and the
/// difference is load-bearing: `gh pr update-branch` decides whether to act
/// from the PR's `mergeStateStatus`, and #1577 KTD-1 established that this
/// repository's ruleset (`strict_required_status_checks_policy: false`,
/// `bypass_mode: always`) makes GitHub report `CLEAN` on PRs that are behind.
/// Routing the repair through that check would make it a no-op in exactly the
/// case it exists for. The SHA comparison in `is_behind_main` is the authority
/// on "behind"; this endpoint is the authority on "make it not so".
///
/// **`Updated` means accepted, not finished.** The endpoint answers `202
/// Accepted` and performs the merge asynchronously, so a zero exit proves
/// GitHub took the request, never that a commit exists. Every string this
/// module renders for `Updated` says "accepted" for that reason. The failure it
/// leaves open — GitHub accepts, the async job fails, no commit and no webhook —
/// resolves the next time `main` moves, because that is a new claim key.
pub(crate) async fn attempt_update_branch(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> UpdateBranchOutcome {
    let endpoint = format!("repos/{repo}/pulls/{pr_number}/update-branch");
    let args = vec!["api", "--method", "PUT", &endpoint];

    match run_gh_subprocess(&args, token).await {
        Ok(_) => UpdateBranchOutcome::Updated,
        Err(e) => classify_update_branch_error(&e),
    }
}

/// Discriminate an update-branch failure from the `gh` error text.
///
/// Pure so the four outcomes can be pinned against real `gh` messages without
/// a subprocess.
pub(crate) fn classify_update_branch_error(err: &str) -> UpdateBranchOutcome {
    let lower = err.to_lowercase();

    // Checked FIRST. A "nothing to do" 422 body can carry the word "merge", and
    // reading it as a conflict would block a PR that has nothing wrong with it —
    // the expensive direction of this mistake.
    if lower.contains("up to date")
        || lower.contains("up-to-date")
        || lower.contains("not behind")
        || lower.contains("no new commits")
    {
        return UpdateBranchOutcome::AlreadyUpToDate;
    }

    if lower.contains("conflict") {
        return UpdateBranchOutcome::Conflict(err.to_string());
    }

    UpdateBranchOutcome::Failed(err.to_string())
}

/// The base branch `is_behind_main` measures against (#1577 resolves
/// `refs/heads/main` unconditionally). A PR based on anything else is outside
/// what that comparison can describe.
pub(crate) const BEHIND_MAIN_BASE_BRANCH: &str = "main";

/// Run the behind-main repair for one PR and report what happened.
///
/// Shared by all three behind-main sites so the base-branch guard, the
/// anti-thrash claim, the repair, and the trace have exactly one shape. `site`
/// names the caller in the log line (R7).
///
/// # Where this sits in each gate, and why
///
/// Every site runs, in order: PR state → forge-gate perimeter → CI checks (a
/// failure stops here) → **this** → merge. Two orderings are load-bearing.
///
/// The perimeter comes first because this function *writes*: it pushes a merge
/// commit onto the PR's head. While the gate only declined, letting a
/// DECISION-CORE PR reach it was harmless; now it would mean the loop touching
/// a branch the operator owns before the operator has been told the PR is
/// theirs to merge.
///
/// The CI-failure check comes first because reporting "CI is red" is strictly
/// more useful than "branch updated, awaiting fresh CI" when both are true —
/// and updating the branch of a red PR buys a CI cycle that will fail again.
/// `ci_success_handler` already had this order; the other two were brought to
/// match it rather than the reverse.
pub(crate) async fn remediate_behind_main(
    site: &'static str,
    pr_number: u64,
    repo: &str,
    base_ref_name: &str,
    token: &str,
    info: &BehindMainInfo,
) -> BehindMainRemediation {
    // Guard before the claim: `is_behind_main` compares against `main` whatever
    // the PR's real base, so for a stacked or release-branch PR "behind" is a
    // misread, not a fact. Declining on a misread was noise; repairing one would
    // push a merge commit onto the head every time `main` moved, re-firing CI
    // and QA each round. An empty name means the field did not come back — treat
    // that as unknown and decline rather than guess.
    if base_ref_name != BEHIND_MAIN_BASE_BRANCH {
        let remediation = BehindMainRemediation::BaseNotMain(if base_ref_name.is_empty() {
            "the PR's base branch could not be read".to_string()
        } else {
            format!("the PR's base branch is `{base_ref_name}`, not `main`")
        });
        log_behind_main_remediation(site, pr_number, repo, info, &remediation);
        return remediation;
    }

    if !claim_update_branch_attempt(pr_number, repo, &info.current_main_sha) {
        let remediation = BehindMainRemediation::AlreadyAttempted;
        log_behind_main_remediation(site, pr_number, repo, info, &remediation);
        return remediation;
    }

    let remediation = match attempt_update_branch(pr_number, repo, token).await {
        UpdateBranchOutcome::Updated => BehindMainRemediation::Updated,
        UpdateBranchOutcome::AlreadyUpToDate => {
            reconcile_already_up_to_date(pr_number, repo, token)
                .await
                .unwrap_or_else(BehindMainRemediation::Contradiction)
        }
        UpdateBranchOutcome::Conflict(detail) => BehindMainRemediation::Conflict(detail),
        UpdateBranchOutcome::Failed(detail) => BehindMainRemediation::Failed(detail),
    };

    // An attempt that never reached GitHub made no commit, so it is not the
    // thrash the cap exists to stop — see `release_update_branch_attempt`.
    if matches!(remediation, BehindMainRemediation::Failed(_)) {
        release_update_branch_attempt(pr_number, repo, &info.current_main_sha);
    }

    log_behind_main_remediation(site, pr_number, repo, info, &remediation);
    remediation
}

/// GitHub said the branch was already up to date; our SHA read said otherwise.
///
/// Re-read the PR's `baseRefOid` rather than believing either side. Note the
/// re-read cannot reuse the stale `pr_base_sha` we came in with: that value is
/// what the contradiction is *about*. Only a genuinely up-to-date PR is allowed
/// back into the gate — proceeding on an unverified assumption is what #1577
/// exists to stop.
async fn reconcile_already_up_to_date(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<BehindMainRemediation, String> {
    let preflight = run_gh_pr_view(pr_number, repo, token).await.map_err(|e| {
        format!(
            "update-branch reported the branch was already up to date; \
             re-reading the PR base failed: {}",
            e.message
        )
    })?;

    match is_behind_main(&preflight.base_ref_oid, repo, token).await {
        Ok(None) => Ok(BehindMainRemediation::NotBehind),
        Ok(Some(fresh)) => Err(format!(
            "update-branch reported the branch was already up to date, but the PR base \
             is still behind main (base: {}, main HEAD: {})",
            fresh.pr_base_sha, fresh.current_main_sha
        )),
        Err(e) => Err(format!(
            "update-branch reported the branch was already up to date; \
             re-checking behind-main failed: {e}"
        )),
    }
}

/// Emit the structured trace for one behind-main decision (R7).
///
/// One emission point for all three sites: a PR that shows up behind without a
/// following `outcome="updated"` is the starvation signal the ticket asked for,
/// and that reading only works if every site writes the same event shape.
///
/// `$MIKA_SPIRIT_LOG_FILE` no longer double-writes every line (mika#2195), but
/// any count taken over lines written before that deploy is doubled — dedup
/// before counting historical windows.
fn log_behind_main_remediation(
    site: &'static str,
    pr_number: u64,
    repo: &str,
    info: &BehindMainInfo,
    remediation: &BehindMainRemediation,
) {
    let (outcome, detail) = match remediation {
        BehindMainRemediation::Updated => ("updated", None),
        BehindMainRemediation::AlreadyAttempted => ("already_attempted", None),
        BehindMainRemediation::NotBehind => ("not_behind", None),
        BehindMainRemediation::Conflict(d) => ("conflict", Some(d.as_str())),
        BehindMainRemediation::Failed(d) => ("failed", Some(d.as_str())),
        BehindMainRemediation::Contradiction(d) => ("contradiction", Some(d.as_str())),
        BehindMainRemediation::BaseNotMain(d) => ("base_not_main", Some(d.as_str())),
    };

    info!(
        event = "behind_main_update_branch",
        site,
        pr_number,
        repo,
        pr_base_sha = %info.pr_base_sha,
        target_main_sha = %info.current_main_sha,
        outcome,
        detail,
        issue = 2238,
        "Behind-main remediation decided for PR"
    );
}

/// Map a remediation outcome to the tool's structured result.
///
/// `None` means "no disposition — let the merge gate continue", and it is the
/// ONLY value on this path that can reach `run_gh_merge`. R3 ("no merge in the
/// same turn as an update-branch") is therefore a property of this one pure
/// function rather than of three call sites that each have to remember it.
///
/// A credential-scope failure takes the mika#1616 branch rather than the
/// generic behind-main one (plan KTD-5). The two are not interchangeable: a 403
/// on update-branch is a config gap with a named remedy — install the App on
/// this repo, or widen the PAT — and mika#1616 exists precisely so the agent
/// reports that instead of paraphrasing an opaque error into a guess about PR
/// state. Every other failure stays `blocked[behind_main]`, so "behind and not
/// repaired" remains one place to look.
pub(crate) fn disposition_for_remediation(
    remediation: &BehindMainRemediation,
    repo: &str,
    info: &BehindMainInfo,
) -> Option<MergeGateResult> {
    if let BehindMainRemediation::Failed(detail) = remediation
        && let Some(credential_scope) = classify_credential_scope_error(detail, repo)
    {
        return Some(credential_scope);
    }

    match remediation {
        BehindMainRemediation::NotBehind => None,
        BehindMainRemediation::Updated => Some(MergeGateResult::BranchUpdated {
            pr_base_sha: info.pr_base_sha.clone(),
            new_main_sha: info.current_main_sha.clone(),
        }),
        BehindMainRemediation::AlreadyAttempted => Some(MergeGateResult::Blocked {
            reason: BlockReason::BehindMain {
                pr_base_sha: info.pr_base_sha.clone(),
                current_main_sha: info.current_main_sha.clone(),
            },
            failing_checks: vec![],
            detail: format!(
                "PR is behind main (base: {}, main HEAD: {}) — an automatic branch update \
                 toward this main HEAD was already attempted and is not being retried.",
                info.pr_base_sha, info.current_main_sha
            ),
        }),
        BehindMainRemediation::Conflict(d) => Some(MergeGateResult::Blocked {
            reason: BlockReason::MergeConflict,
            failing_checks: vec![],
            detail: format!(
                "PR is behind main and the automatic branch update hit a conflict — \
                 resolution is required before merging. {d}"
            ),
        }),
        BehindMainRemediation::Failed(d) | BehindMainRemediation::Contradiction(d) => {
            Some(MergeGateResult::Blocked {
                reason: BlockReason::BehindMain {
                    pr_base_sha: info.pr_base_sha.clone(),
                    current_main_sha: info.current_main_sha.clone(),
                },
                failing_checks: vec![],
                detail: format!(
                    "PR is behind main (base: {}, main HEAD: {}) and the automatic branch \
                     update did not go through: {d}",
                    info.pr_base_sha, info.current_main_sha
                ),
            })
        }
        BehindMainRemediation::BaseNotMain(d) => Some(MergeGateResult::GateError {
            kind: GateErrorKind::Unknown,
            detail: format!(
                "Behind-main could not be evaluated for this PR: {d}. The behind-main \
                 comparison resolves `refs/heads/main` unconditionally (#1577), so its \
                 verdict does not describe a PR based on another branch. The gate did not \
                 merge and did not touch the branch — an operator decides this one."
            ),
        }),
    }
}

/// The remediation-specific half of the two handlers' enrichment text.
///
/// `None` means "no enrichment — let the handler continue", the webhook mirror
/// of `disposition_for_remediation`'s `None`. Shared because the load-bearing
/// sentence is the do-not-merge instruction, and two copies of it would drift.
///
/// The wording deliberately avoids the completion-claim guard's vocabulary
/// (`merged` / `deployed` / `complete(d)` / `shipped`) — these strings are
/// pre-digest input to an LLM turn, and a pre-digest that trips that guard
/// costs the turn.
pub(crate) fn describe_behind_main_remediation(
    remediation: &BehindMainRemediation,
    info: &BehindMainInfo,
) -> Option<String> {
    let base = &info.pr_base_sha;
    let head = &info.current_main_sha;

    match remediation {
        BehindMainRemediation::NotBehind => None,
        BehindMainRemediation::Updated => Some(format!(
            "the PR was behind main (base: {base}, main HEAD: {head}). GitHub has accepted an \
             automatic branch update toward {head} (mika#2238), which moves the PR's head to a \
             new commit. Do NOT merge this PR in this turn and do NOT call \
             `pr_merge_with_gate` for it: that new head commit has no CI result, and merging \
             it would put an unvalidated commit on main (the failure mika#1577 closed). Do NOT \
             rebase by hand. End the turn. **The PR now needs a fresh QA review** — the head \
             SHA moved, so the existing approval no longer matches HEAD and the stale-SHA gate \
             will hold the PR until QA re-reviews the updated head. Say so when you notify: \
             the mechanical behind-main state is repaired, the review is not.\n\n"
        )),
        BehindMainRemediation::AlreadyAttempted => Some(format!(
            "the PR is behind main (base: {base}, main HEAD: {head}). An automatic branch \
             update toward this exact main HEAD was already accepted by GitHub, so it is not \
             being re-sent (anti-thrash guard, mika#2238). Do NOT merge and do NOT rebase by \
             hand. End the turn; if the PR's head never moves, surface it to the operator.\n\n"
        )),
        BehindMainRemediation::Conflict(d) => Some(format!(
            "the PR is behind main (base: {base}, main HEAD: {head}) and the automatic branch \
             update hit a real conflict: {d}. Do NOT merge. Conflict resolution is required \
             before this PR can go in.\n\n"
        )),
        BehindMainRemediation::Failed(d) | BehindMainRemediation::Contradiction(d) => {
            Some(format!(
                "the PR is behind main (base: {base}, main HEAD: {head}) and the automatic \
                 branch update did not go through: {d}. Do NOT merge. Do NOT rebase by hand — \
                 surface the failure to the operator with that detail verbatim. If it names a \
                 403 or `Resource not accessible by integration`, the merge credential lacks \
                 write access to this repository: the fix is to install the mika GitHub App on \
                 it with Contents + Pull requests write permission, or to grant the configured \
                 PAT the `repo` scope.\n\n"
            ))
        }
        BehindMainRemediation::BaseNotMain(d) => Some(format!(
            "the behind-main comparison could not be evaluated for this PR: {d}. That \
             comparison resolves `refs/heads/main` unconditionally (#1577), so it does not \
             describe a PR based on another branch. The gate took no action and did not touch \
             the branch. Do NOT merge and do NOT rebase by hand. Surface it to the \
             operator.\n\n"
        )),
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

/// Detect credential-scope failures from a `gh` error string (mika#1616).
///
/// When mika-dev's merge credential (GitHub App installation token, or PAT
/// fallback) is not authorized to write to the target repo — e.g. the App is
/// installed on `senara-solutions/mika` but not `senara-solutions/mika-cloud`
/// — `gh pr merge` fails with a 403 / "Resource not accessible by integration"
/// / forbidden response. The bare exit code carries no cause, so the LLM
/// paraphrases it into fabricated guesses ("PAT gap blocks pr_merge_with_gate").
///
/// This classifier returns an actionable `GateError` naming the repo and the
/// concrete remediation so the agent surfaces a real cause. Detection mirrors
/// the established `classify_gh_error()` heuristic in `builtin_handlers.rs`
/// (403 / forbidden / "resource not accessible"). Returns `None` for any other
/// failure so unrelated errors keep their existing classification.
pub(crate) fn classify_credential_scope_error(err: &str, repo: &str) -> Option<MergeGateResult> {
    let lower = err.to_lowercase();
    let is_credential_scope = lower.contains("resource not accessible")
        || lower.contains("must have admin rights")
        || lower.contains("403")
        || lower.contains("forbidden");

    if !is_credential_scope {
        return None;
    }

    Some(MergeGateResult::GateError {
        kind: GateErrorKind::CredentialScope {
            repo: repo.to_string(),
        },
        detail: format!(
            "Merge credential lacks write access to {repo}. The GitHub App \
             installation (or PAT) used for autonomous merges is not authorized \
             to merge pull requests on this repository. This is a credential/config \
             gap, not a PR-state problem. Fix: install the mika GitHub App on \
             {repo} with Contents + Pull requests write permission, or grant the \
             configured PAT the `repo` scope for private repositories. \
             Underlying gh error: {err}"
        ),
    })
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

    // -----------------------------------------------------------------------
    // Behind-main remediation (mika#2238)
    // -----------------------------------------------------------------------

    const REPO: &str = "senara-solutions/mika";

    fn behind_info() -> BehindMainInfo {
        BehindMainInfo {
            pr_base_sha: "abc1234deadbeef".to_string(),
            current_main_sha: "def5678cafebabe".to_string(),
        }
    }

    // -- U1: the four outcomes, discriminated from real `gh` error text --

    #[test]
    fn mika2238_conflict_error_classifies_as_conflict() {
        // The 422 body GitHub returns when the update cannot be applied.
        let err = "gh exit code 1: gh: merge conflict between base and head (HTTP 422)";
        assert_eq!(
            classify_update_branch_error(err),
            UpdateBranchOutcome::Conflict(err.to_string())
        );
    }

    #[test]
    fn mika2238_already_up_to_date_is_not_read_as_a_conflict() {
        // The benign race: main moved, or someone else updated the branch,
        // between our SHA read and this call. GitHub's wording for it can carry
        // the word "merge", and reading it as a conflict would block a PR that
        // has nothing wrong with it — the expensive direction of the mistake.
        for err in [
            "gh exit code 1: gh: This branch is already up to date with the base branch (HTTP 422)",
            "gh exit code 1: gh: merge branch is already up-to-date (HTTP 422)",
            "gh exit code 1: gh: the head branch is not behind the base branch (HTTP 422)",
            "gh exit code 1: gh: no new commits on the base branch (HTTP 422)",
        ] {
            assert_eq!(
                classify_update_branch_error(err),
                UpdateBranchOutcome::AlreadyUpToDate,
                "expected AlreadyUpToDate for: {err}"
            );
        }
    }

    #[test]
    fn mika2238_permission_and_missing_gh_classify_as_failed() {
        for err in [
            "gh exit code 1: gh: Resource not accessible by integration (HTTP 403)",
            "gh exit code 1: gh: Not Found (HTTP 404)",
            "gh CLI not found — install from https://cli.github.com",
        ] {
            assert_eq!(
                classify_update_branch_error(err),
                UpdateBranchOutcome::Failed(err.to_string()),
                "expected Failed for: {err}"
            );
        }
    }

    // -- U1/R5: the new first-level action --

    #[test]
    fn mika2238_serialize_branch_updated() {
        let info = behind_info();
        let result = MergeGateResult::BranchUpdated {
            pr_base_sha: info.pr_base_sha.clone(),
            new_main_sha: info.current_main_sha.clone(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "branch_updated");
        assert_eq!(json["pr_base_sha"], info.pr_base_sha);
        assert_eq!(json["new_main_sha"], info.current_main_sha);
        // A repaired behind-main state is NOT a blockage — an agent that sees
        // `blocked` here would report a failure where the loop is progressing.
        assert!(json.get("reason").is_none());
    }

    // -- U3/R3: the test the plan calls the most important one --

    #[test]
    fn mika2238_successful_update_never_falls_through_to_the_merge() {
        // `None` is the ONLY value on this path that lets the caller reach
        // `run_gh_merge`. An update-branch creates a new head commit whose CI
        // has not run; merging it in the same turn is exactly the failure
        // mika#1577 was written to close, re-opened through mika#2238's door.
        let info = behind_info();
        let disposition = disposition_for_remediation(&BehindMainRemediation::Updated, REPO, &info);

        assert!(
            disposition.is_some(),
            "an updated branch must terminate the turn, never continue into the merge gate"
        );
        assert_eq!(
            disposition,
            Some(MergeGateResult::BranchUpdated {
                pr_base_sha: info.pr_base_sha.clone(),
                new_main_sha: info.current_main_sha.clone(),
            })
        );
    }

    #[test]
    fn mika2238_only_a_not_behind_pr_continues_into_the_merge_gate() {
        let info = behind_info();
        let continues =
            |r: BehindMainRemediation| disposition_for_remediation(&r, REPO, &info).is_none();

        assert!(
            continues(BehindMainRemediation::NotBehind),
            "a PR that is genuinely not behind must not be held by this path"
        );
        for held in [
            BehindMainRemediation::Updated,
            BehindMainRemediation::AlreadyAttempted,
            BehindMainRemediation::Conflict("merge conflict".to_string()),
            BehindMainRemediation::Failed("HTTP 403".to_string()),
            BehindMainRemediation::Contradiction("still behind".to_string()),
            BehindMainRemediation::BaseNotMain("release/1.4".to_string()),
        ] {
            assert!(
                !continues(held.clone()),
                "{held:?} must terminate the turn, not continue into the merge gate"
            );
        }
    }

    #[test]
    fn mika2238_conflict_blocks_as_a_merge_conflict_not_as_behind_main() {
        // The ticket's second bullet: BEHIND is mechanical, a conflict is not.
        // They must stay distinguishable or the agent cannot tell "wait" from
        // "someone has to resolve this".
        let info = behind_info();
        let detail = "gh: merge conflict between base and head (HTTP 422)";
        let disposition = disposition_for_remediation(
            &BehindMainRemediation::Conflict(detail.to_string()),
            REPO,
            &info,
        )
        .expect("a conflict must terminate the turn");

        match disposition {
            MergeGateResult::Blocked {
                reason, detail: d, ..
            } => {
                assert_eq!(reason, BlockReason::MergeConflict);
                assert!(d.contains(detail), "the gh detail must survive: {d}");
            }
            other => panic!("expected blocked[merge_conflict], got {other:?}"),
        }
    }

    #[test]
    fn mika2238_failed_update_degrades_to_behind_main_never_to_a_merge() {
        // R8: a failed update leaves the gate at least as closed as before.
        let info = behind_info();
        let disposition = disposition_for_remediation(
            &BehindMainRemediation::Failed("gh exit code 1: Not Found (HTTP 404)".to_string()),
            REPO,
            &info,
        )
        .expect("a failed update must terminate the turn");

        match disposition {
            MergeGateResult::Blocked {
                reason, detail: d, ..
            } => {
                assert_eq!(
                    reason,
                    BlockReason::BehindMain {
                        pr_base_sha: info.pr_base_sha.clone(),
                        current_main_sha: info.current_main_sha.clone(),
                    }
                );
                assert!(d.contains("404"), "the failure cause must survive: {d}");
            }
            other => panic!("expected blocked[behind_main], got {other:?}"),
        }
    }

    #[test]
    fn mika2238_permission_failure_takes_the_credential_scope_branch() {
        // KTD-5. A 403 on update-branch is a config gap with a named remedy,
        // not a fact about the PR. mika#1616 built that branch so the agent
        // reports the remedy instead of paraphrasing an opaque error — routing
        // it into the generic behind-main detail would throw that away.
        let info = behind_info();
        let disposition = disposition_for_remediation(
            &BehindMainRemediation::Failed(
                "gh exit code 1: gh: Resource not accessible by integration (HTTP 403)".to_string(),
            ),
            REPO,
            &info,
        )
        .expect("a permission failure must terminate the turn");

        match disposition {
            MergeGateResult::GateError {
                kind, detail: d, ..
            } => {
                assert_eq!(
                    kind,
                    GateErrorKind::CredentialScope {
                        repo: REPO.to_string()
                    }
                );
                assert!(
                    d.contains("install the mika GitHub App"),
                    "the remedy must reach the agent: {d}"
                );
            }
            other => panic!("expected gate_errored[credential_scope], got {other:?}"),
        }
    }

    // -- U2/R4: the anti-thrash cap --

    #[test]
    fn mika2238_second_pass_on_the_same_target_sha_does_not_re_emit() {
        let map = DashMap::new();
        let now = Instant::now();
        assert!(
            claim_update_attempt_in(&map, 2236, "senara-solutions/mika", "main-sha-one", now),
            "the first caller must get the attempt"
        );
        assert!(
            !claim_update_attempt_in(&map, 2236, "senara-solutions/mika", "main-sha-one", now),
            "a second arrival at the same target SHA must not re-emit an update"
        );
    }

    #[test]
    fn mika2238_a_new_main_sha_legitimately_re_opens_one_attempt() {
        // KTD-2: the key is the SHA aimed at, not a counter. When another PR
        // merges, main moves and this PR is behind something new — a fresh,
        // legitimate attempt, not thrash.
        let map = DashMap::new();
        let now = Instant::now();
        assert!(claim_update_attempt_in(
            &map,
            2236,
            "senara-solutions/mika",
            "main-sha-one",
            now
        ));
        assert!(
            claim_update_attempt_in(&map, 2236, "senara-solutions/mika", "main-sha-two", now),
            "a different main HEAD is a different target and re-opens one attempt"
        );
    }

    #[test]
    fn mika2238_the_cap_is_scoped_to_one_pr() {
        let map = DashMap::new();
        let now = Instant::now();
        assert!(claim_update_attempt_in(
            &map,
            2236,
            "senara-solutions/mika",
            "sha",
            now
        ));
        assert!(
            claim_update_attempt_in(&map, 2237, "senara-solutions/mika", "sha", now),
            "one PR's attempt must not spend another PR's"
        );
        assert!(
            claim_update_attempt_in(&map, 2236, "senara-solutions/mika-cloud", "sha", now),
            "the same PR number in another repo is another PR"
        );
    }

    #[test]
    fn mika2238_concurrent_claims_on_one_key_yield_exactly_one_attempt() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let map = Arc::new(DashMap::new());
        let granted = Arc::new(AtomicUsize::new(0));
        let now = Instant::now();

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let map = Arc::clone(&map);
                let granted = Arc::clone(&granted);
                std::thread::spawn(move || {
                    if claim_update_attempt_in(&map, 2236, "senara-solutions/mika", "sha", now) {
                        granted.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            granted.load(Ordering::SeqCst),
            1,
            "exactly one of five concurrent callers may emit the update"
        );
    }

    #[test]
    fn mika2238_attempt_ledger_stays_bounded() {
        let map = DashMap::new();
        let base = Instant::now();
        for i in 0..UPDATE_ATTEMPT_CAP {
            assert!(claim_update_attempt_in(
                &map,
                i as u64,
                "senara-solutions/mika",
                "sha",
                base
            ));
        }
        assert_eq!(map.len(), UPDATE_ATTEMPT_CAP);

        let later = base + UPDATE_ATTEMPT_TTL + Duration::from_secs(1);
        assert!(claim_update_attempt_in(
            &map,
            999_999,
            "senara-solutions/mika",
            "sha",
            later
        ));
        assert!(
            map.len() < UPDATE_ATTEMPT_CAP,
            "entries older than the TTL must be swept, got {}",
            map.len()
        );
    }

    #[test]
    fn mika2238_a_failed_attempt_gives_its_claim_back() {
        // The claim is spent optimistically, before the call, so two concurrent
        // webhooks cannot both fire an update. But an attempt that never reached
        // GitHub made no commit, so keeping it spent would strand the PR: nothing
        // evicts the entry until the ledger hits its soft cap, and with a
        // serialized dispatch `main` may not move for hours. A guard that
        // manufactures a permanent stall is the defect this ticket exists to end.
        let sha = "mika2238-release-unique-sha-7c2e5a";
        assert!(claim_update_branch_attempt(4242, REPO, sha));
        assert!(
            !claim_update_branch_attempt(4242, REPO, sha),
            "the claim must be exclusive while it is held"
        );

        release_update_branch_attempt(4242, REPO, sha);

        assert!(
            claim_update_branch_attempt(4242, REPO, sha),
            "a released claim must be re-takeable — otherwise one transient gh \
             failure blocks this PR until `main` moves"
        );
    }

    #[test]
    fn mika2238_a_contradiction_detail_never_reaches_the_credential_classifier() {
        // `classify_credential_scope_error` matches the bare substring `403`,
        // and a 40-hex SHA contains it ~1% of the time. The reconcile path
        // composes prose around two SHAs, so routing it through that helper
        // would occasionally answer a state contradiction with a confident,
        // wrong "install the GitHub App". Hence the separate variant.
        let info = BehindMainInfo {
            pr_base_sha: "a403bc1234567890abcdef1234567890abcdef12".to_string(),
            current_main_sha: "def5678cafebabe".to_string(),
        };
        let contradiction = BehindMainRemediation::Contradiction(format!(
            "update-branch reported the branch was already up to date, but the PR base \
             is still behind main (base: {}, main HEAD: {})",
            info.pr_base_sha, info.current_main_sha
        ));

        // Sanity: the composed detail really does contain the substring that
        // would trip the classifier, so this test would catch the regression.
        let detail = match &contradiction {
            BehindMainRemediation::Contradiction(d) => d.clone(),
            other => panic!("expected Contradiction, got {other:?}"),
        };
        assert!(classify_credential_scope_error(&detail, REPO).is_some());

        match disposition_for_remediation(&contradiction, REPO, &info)
            .expect("a contradiction must terminate the turn")
        {
            MergeGateResult::Blocked { reason, .. } => assert_eq!(
                reason,
                BlockReason::BehindMain {
                    pr_base_sha: info.pr_base_sha.clone(),
                    current_main_sha: info.current_main_sha.clone(),
                }
            ),
            other => panic!("expected blocked[behind_main], got {other:?}"),
        }
    }

    #[test]
    fn mika2238_a_pr_based_on_another_branch_is_never_repaired() {
        // `is_behind_main` resolves `refs/heads/main` unconditionally (#1577),
        // so for a stacked or release-branch PR "behind" is a misread. Declining
        // on a misread was noise; repairing one would push a merge commit onto
        // the head every time `main` moved, re-firing CI and QA each round.
        let info = behind_info();
        for base in ["release/1.4", "feat/parent-of-a-stack", ""] {
            let remediation = BehindMainRemediation::BaseNotMain(base.to_string());
            let disposition = disposition_for_remediation(&remediation, REPO, &info)
                .expect("a non-main base must terminate the turn, never merge");
            assert!(
                matches!(disposition, MergeGateResult::GateError { .. }),
                "a base the comparison cannot describe is a gate error, not a PR-state \
                 claim; got {disposition:?}"
            );
        }
    }

    #[test]
    fn mika2238_public_claim_uses_the_process_global_ledger() {
        // Exercise the real path once. Unique key so parallel tests sharing the
        // process-global map cannot collide.
        let sha = "mika2238-public-api-unique-sha-4b1c9d";
        assert!(claim_update_branch_attempt(
            2238,
            "senara-solutions/mika",
            sha
        ));
        assert!(!claim_update_branch_attempt(
            2238,
            "senara-solutions/mika",
            sha
        ));
    }

    // -- R1: one call site, and only one --

    /// Fails if update-branch is invoked anywhere but [`attempt_update_branch`].
    ///
    /// A behavioural test cannot hold this: a second call site would make no
    /// assertion fail — it would only make the anti-thrash guard bypassable,
    /// which is invisible until a PR is being hammered in production. Same
    /// shape as `grooming_marker::tests::no_grooming_regex_outside_this_module`
    /// and for the same reason: the regression would not produce a wrong
    /// answer, it would produce an unguarded one.
    #[test]
    fn mika2238_update_branch_has_exactly_one_call_site() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("tools/pr_merge_with_gate.rs");

        let mut offenders = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
                panic!("the guard must be able to read {}: {e}", dir.display())
            });
            for entry in entries {
                let path = entry.expect("readable directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this_module {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("the guard must be able to read {}: {e}", path.display())
                });
                scanned += 1;
                for (n, line) in content.lines().enumerate() {
                    let trimmed = line.trim_start();
                    // Prose (doc comments, `//` comments) may name the endpoint;
                    // only executable code that builds the argv is an offence.
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    if line.contains("update-branch") || line.contains("/update-branch") {
                        offenders.push(format!(
                            "{}:{}: {}",
                            path.strip_prefix(&src_root).unwrap_or(&path).display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }

        assert!(scanned > 0, "the guard scanned no files — broken path");
        assert!(
            offenders.is_empty(),
            "mika#2238 — update-branch is invoked outside `attempt_update_branch`. \
             R1 makes that helper the single call site so the per-(PR, main SHA) \
             anti-thrash claim cannot be bypassed. Route the call through it:\n{}",
            offenders.join("\n")
        );
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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
            base_ref_name: "main".to_string(),
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

    // -- Credential-scope classification tests (mika#1616) --

    #[test]
    fn serialize_gate_error_credential_scope() {
        let result = MergeGateResult::GateError {
            kind: GateErrorKind::CredentialScope {
                repo: "senara-solutions/mika-cloud".to_string(),
            },
            detail: "Merge credential lacks write access".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert_eq!(json["kind"]["kind"], "credential_scope");
        assert_eq!(json["kind"]["repo"], "senara-solutions/mika-cloud");
    }

    #[test]
    fn classify_credential_scope_resource_not_accessible() {
        // The exact string GitHub App installation tokens return when the App
        // lacks write access to the target repo — the mika#1616 root cause.
        let err = "gh exit code 1: HTTP 403: Resource not accessible by integration";
        let result = classify_credential_scope_error(err, "senara-solutions/mika-cloud");
        match result {
            Some(MergeGateResult::GateError {
                kind: GateErrorKind::CredentialScope { repo },
                detail,
            }) => {
                assert_eq!(repo, "senara-solutions/mika-cloud");
                // Actionable diagnostic: names the repo + remediation.
                assert!(detail.contains("senara-solutions/mika-cloud"));
                assert!(detail.contains("GitHub App"));
                // Preserves the underlying gh error for debugging.
                assert!(detail.contains("Resource not accessible by integration"));
            }
            other => panic!("Expected CredentialScope GateError, got: {other:?}"),
        }
    }

    #[test]
    fn classify_credential_scope_forbidden_and_403() {
        for err in [
            "gh exit code 1: HTTP 403: Forbidden",
            "gh: 403 Forbidden accessing the merge endpoint",
            "You must have admin rights to this repository",
        ] {
            assert!(
                classify_credential_scope_error(err, "owner/repo").is_some(),
                "expected credential-scope classification for: {err}"
            );
        }
    }

    #[test]
    fn classify_credential_scope_ignores_unrelated_errors() {
        // Non-credential failures must fall through to their existing
        // classification — no false positives that would mask real problems.
        for err in [
            "gh exit code 1: no checks reported on the 'main' branch",
            "GraphQL: Pull request is in draft state",
            "merge conflict between base and head",
            "Connection refused",
        ] {
            assert!(
                classify_credential_scope_error(err, "owner/repo").is_none(),
                "expected no credential-scope classification for: {err}"
            );
        }
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
            gateway_url: None,
            internal_token: None,
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
            tier: mika_common::home::AgentTier::Default,
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
