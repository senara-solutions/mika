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
                EVERY check on the PR head — not only the ones branch protection marks as \
                required. If any check is failing or cancelled, the merge is blocked and the \
                failing checks are named. If any check is still running (but none failing), \
                the merge is blocked as 'checks_pending' and the gate WAITS — it does not \
                enable GitHub auto-merge, which would fire on required checks only. If every \
                check passed or was skipped, the PR is merged immediately.\n\n\
                IMPORTANT: blocked/'checks_pending' is a HOLD, not a failure. Nothing is \
                wrong with the PR; its CI is still running. Do NOT retry in a loop and do NOT \
                fall back to `run_gh pr merge`. End the turn — the merge re-enters by itself \
                when the next `check_suite.completed(success)` arrives.\n\n\
                IMPORTANT: 'branch_updated' means the PR was behind main and GitHub accepted an \
                update of its branch. No merge was attempted. Do NOT call this tool again for \
                this PR in the same turn and do NOT rebase by hand — the update moves the head \
                to a new commit with no CI result. The PR then needs a FRESH QA review: moving \
                the head SHA invalidates the approval that pointed at the old one. End the \
                turn and say that the behind-main state is repaired but the review is not.\n\n\
                After a successful merge (action: 'merged'), update the task status before \
                reporting to the user.\n\n\
                Returns a structured JSON response with an 'action' field. Possible actions: \
                'merged', 'blocked', 'already_merged', 'gate_errored', 'branch_updated'. \
                ('auto_merge_enabled' is a retired action kept for old records — no call \
                produces it any more.) Branch on 'action' to determine next steps."
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
        // -- Step 0: authority — the reviewer is never a merge actor (mika#2248) --
        //
        // Placed before input validation and before the token check on purpose:
        // this is not about the PR, it is about who is asking. `mergedBy` is
        // stamped with the identity of whatever token `run_gh_merge` injects, so
        // an agent that approved the PR must be refused here rather than gated
        // later — otherwise the refusal depends on the PR's state, and a green
        // mechanical PR would slip through. Measured on mika#2244:
        // `mergedBy = mika-platform-qa` on a PR the same agent had approved.
        let agent_id = ctx.db.agent_id();
        if mika_common::forge_identity::is_reviewer_agent(agent_id) {
            let result = MergeGateResult::Blocked {
                reason: BlockReason::ReviewerCannotMerge {
                    agent_id: agent_id.to_string(),
                    dispatcher_agent: mika_common::forge_identity::DISPATCHER_AGENT.to_string(),
                },
                failing_checks: vec![],
                detail: format!(
                    "`{agent_id}` is the autonomous reviewer and cannot merge: the forge would \
                     record the merge under the same login that posted the review. The dispatcher \
                     `{}` owns the merge — post the verdict and let the merge-ready path hand it \
                     over (mika#2248).",
                    mika_common::forge_identity::DISPATCHER_AGENT,
                ),
            };
            warn!(
                event = "pr_merge_with_gate_reviewer_refused",
                agent_id,
                "pr_merge_with_gate called by the reviewer agent — refused before any gh call (mika#2248)"
            );
            return emit_gate_result(result, ctx).await;
        }

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
                // mika#1964 — this is the leak M1 made reachable: `pr_merge_with_gate`
                // is registered by `default_tools()`, so it sits in EVERY agent's tool
                // array whatever its tier, and `FAMILY_IDENTITY` declares no
                // `[tools].disabled` block. The old string was therefore served to a
                // family tenant, and the ticket's "currently latent" premise was false.
                return Ok(crate::tools::dispatch_substrate_unavailable(
                    "Merging pull requests is not available right now. Nothing was \
                     merged. This is not something retrying will fix — report it.",
                    // substrate-diagnostic: the operator channel — the variable and
                    // the App are the remedy, and only the operator can apply it.
                    "pr_merge_with_gate has no GitHub credential on this agent. \
                     Set MIKA_GITHUB_TOKEN or configure a GitHub App.",
                    "pr_merge_with_gate",
                    ctx,
                )
                .await);
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
                return emit_gate_result(result, ctx).await;
            }
        };

        // Classify preflight result
        if let Some(result) = classify_preflight(&preflight) {
            return emit_gate_result(result, ctx).await;
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
            return emit_gate_result(result, ctx).await;
        }

        // -- Step 2: Fetch EVERY check status of the head (mika#2617) --
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
                return emit_gate_result(result, ctx).await;
            }
        };

        // -- Step 3: Decide (pure) and act --
        //
        // A bucket outside the known vocabulary is reported and left
        // non-blocking (mika#2617, plan R6): refusing on it would break merging
        // on another `gh` version, i.e. trade a red `main` for a stopped loop.
        // Nothing is emitted on the nominal path (doctrine mika#2131).
        for (name, bucket) in unknown_buckets(&checks) {
            warn!(
                event = "merge_gate_unknown_check_bucket",
                repo,
                pr = pr_number,
                name,
                bucket,
                "gh returned a check bucket outside the known set — it is NOT blocking (mika#2617)"
            );
        }

        let decision = decide_merge_gate(&checks);

        // -- Step 3a: A red PR is reported as red, before anything else --
        // This arm runs ahead of the behind-main step below so a PR that is both
        // behind and failing reports the failing checks, not "branch updated".
        if let MergeGateDecision::ChecksFailed { failing } = &decision {
            let result = MergeGateResult::Blocked {
                reason: BlockReason::RequiredCheckFailed {
                    failing_checks: failing.clone(),
                },
                failing_checks: failing.clone(),
                detail: format!(
                    "{} check(s) failed on this head: {}",
                    failing.len(),
                    describe_checks(failing)
                ),
            };
            return emit_gate_result(result, ctx).await;
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
                    return emit_gate_result(result, ctx).await;
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

        match decision {
            // Handled above by step 3a, which returns.
            MergeGateDecision::ChecksFailed { .. } => {
                unreachable!("ChecksFailed returns at step 3a")
            }
            MergeGateDecision::ChecksPending { pending } => {
                // mika#2617 U2: refuse and wait, never `--auto`. GitHub's
                // auto-merge fires on the REQUIRED checks alone, so arming it
                // here would hand the merge decision back to the definition of
                // "green" this ticket exists to stop using.
                //
                // mika#1211: persist pr_url on supervisor so the orphan reaper
                // (#871) doesn't flip it to `failed` and the parent-completer
                // (mika#1162) can promote it to `completed` once the dispatch
                // callback ages past REAPER_GRACE_SECONDS. The call MOVED here
                // from the auto-merge arm — it is the same need (a supervisor
                // whose child is waiting on a PR that is open and not merged),
                // and dropping it with the arm would have let a healthy
                // supervisor be reaped 600 s later.
                let pr_url = format!("https://github.com/{repo}/pull/{pr_number}");
                write_pending_pr_url_to_supervisor(ctx, &pr_url).await;

                let result = MergeGateResult::Blocked {
                    reason: BlockReason::ChecksPending {
                        pending_checks: pending.clone(),
                    },
                    failing_checks: vec![],
                    detail: format!(
                        "{} check(s) still running on this head: {}. The gate waits: \
                         GitHub auto-merge fires on required checks only, which is not \
                         what this gate reads (mika#2617). The merge re-enters by itself \
                         on the next `check_suite.completed(success)`.",
                        pending.len(),
                        describe_checks(&pending)
                    ),
                };
                emit_gate_result(result, ctx).await
            }
            MergeGateDecision::AllChecksPassed => {
                // Merge immediately
                let merge_result =
                    run_gh_merge(pr_number, repo, merge_method, delete_branch, token).await;

                // Unify success/error into a single string for "already merged" detection
                let (output, is_err) = match merge_result {
                    Ok(s) => (s, false),
                    Err(s) => (s, true),
                };

                let output_lower = output.to_lowercase();
                if output_lower.contains("already been merged") {
                    let result = MergeGateResult::AlreadyMerged;
                    emit_gate_result(result, ctx).await
                } else if !is_err {
                    let result = MergeGateResult::Merged;
                    emit_gate_result(result, ctx).await
                } else if let Some(result) = classify_credential_scope_error(&output, repo) {
                    // Credential-scope 403 takes priority over the generic
                    // draft/conflict/review classification below (mika#1616).
                    emit_gate_result(result, ctx).await
                } else if output_lower.contains("draft") {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::Draft,
                        failing_checks: vec![],
                        detail: "PR is a draft — convert to ready before merging".to_string(),
                    };
                    emit_gate_result(result, ctx).await
                } else if output_lower.contains("merge conflict")
                    || output_lower.contains("not mergeable")
                {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::MergeConflict,
                        failing_checks: vec![],
                        detail: "PR has merge conflicts — rebase needed".to_string(),
                    };
                    emit_gate_result(result, ctx).await
                } else if output_lower.contains("review") && output_lower.contains("required") {
                    let result = MergeGateResult::Blocked {
                        reason: BlockReason::MissingApproval,
                        failing_checks: vec![],
                        detail: "Required reviews not met".to_string(),
                    };
                    emit_gate_result(result, ctx).await
                } else {
                    let result = MergeGateResult::GateError {
                        kind: GateErrorKind::Unknown,
                        detail: format!("Merge failed: {output}"),
                    };
                    emit_gate_result(result, ctx).await
                }
            }
        }
    }
}

/// The single site that turns a [`MergeGateResult`] into the tool's `ToolOutput`
/// (mika#1964).
///
/// It exists because a diagnostic can only be routed where a `ToolContext` is in
/// hand, and the sixteen `ToolOutput::success(to_string_pretty(&result))` call
/// sites this replaced had no way to do it. `classify_credential_scope_error` and
/// `disposition_for_remediation` stay pure and stay untouched: they compose the
/// operator-shaped text, and the channel decision is taken here, once.
///
/// `GateErrorKind::CredentialScope` keeps its variant and its wire name — mika#1616
/// posed them so the model could branch, and nothing here is a taxonomy change.
/// Only the `detail` changes channel: the model gets a neutral statement that the
/// merge did not happen and the branch was not touched, the operator gets the
/// remedy naming the App and the PAT scope.
///
/// **The result stays a successful, parseable JSON document on every tier.** The
/// LLM branches on `action`, and `tool_execution/dispatch.rs` reads `is_error` as
/// "the call failed" — so the credential-scope refusal is neither turned into an
/// error nor followed by prose. That is why this does not go through
/// `dispatch_substrate_unavailable` (which sets `is_error` and, on operator tier,
/// appends the diagnostic after a blank line, breaking the JSON):
///
/// - **Family / Champion:** the neutral JSON goes to `content`, `detail` goes to
///   the telemetry sink via `attach_substrate_diagnostic` (`is_error` stays false).
/// - **Default:** the operator is the reader, so the result is served whole,
///   byte for byte as before mika#1964 — the remedy already sits inside `detail`.
///
/// The match names every tier (no `_ =>`), for the reason
/// `dispatch_substrate_diagnostic` gives: the next tier must decide, not inherit.
///
/// Every other variant is serialized byte for byte as before.
async fn emit_gate_result(result: MergeGateResult, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    if let MergeGateResult::GateError {
        kind: kind @ GateErrorKind::CredentialScope { .. },
        detail,
    } = result
    {
        match ctx.tier {
            mika_common::home::AgentTier::Family | mika_common::home::AgentTier::Champion => {
                let neutral = MergeGateResult::GateError {
                    kind,
                    detail: CREDENTIAL_SCOPE_NEUTRAL_DETAIL.to_string(),
                };
                let mut out = ToolOutput::success(serde_json::to_string_pretty(&neutral)?);
                crate::tools::attach_substrate_diagnostic(
                    &mut out,
                    detail,
                    "pr_merge_with_gate",
                    ctx,
                )
                .await;
                return Ok(out);
            }
            mika_common::home::AgentTier::Default => {
                let whole = MergeGateResult::GateError { kind, detail };
                return Ok(ToolOutput::success(serde_json::to_string_pretty(&whole)?));
            }
        }
    }
    Ok(ToolOutput::success(serde_json::to_string_pretty(&result)?))
}

/// What the model reads in place of the credential-scope remedy.
///
/// It says *that* the merge did not happen and *that* the branch is untouched —
/// the two facts the turn needs in order not to claim a merge (the mika#483 /
/// mika#1331 family) — and nothing about infrastructure. A fallback that said
/// nothing at all would make the model invent a cause (mika#1783 risk 2).
const CREDENTIAL_SCOPE_NEUTRAL_DETAIL: &str = "This pull request cannot be merged from here: the merge was refused before it \
     was attempted. Nothing was merged and the branch was not modified. This is not \
     a problem with the pull request itself and retrying will not change it — report \
     it rather than working around it.";

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
    /// **No site produces this since 2026-10-02 (mika#2617 U2).**
    ///
    /// Kept as a wire format, not renamed and not removed: a `tasks.result`
    /// persisted before that date can carry it, and the three `self-dev*`
    /// prompts still describe it. What replaced it is
    /// [`BlockReason::ChecksPending`] — a refusal instead of a delegation of
    /// the merge decision to branch protection.
    #[allow(dead_code)]
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
    /// The PR was behind `main`, GitHub accepted an update of its branch
    /// (mika#2238), and the PR's base was then **read** to have moved
    /// (mika#2252). `new_main_sha` is that read value — an acceptance whose
    /// landing was not observed is `blocked[behind_main]`, never this variant.
    /// Deliberately NOT a `Blocked` variant: a behind-main state
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
    /// One or more CI checks failed or were cancelled.
    ///
    /// **The wire name keeps the word "required" and that word is a vestige,
    /// dated here rather than renamed (mika#2617).** It is carried by the
    /// `BlockReason` taxonomy of the three `self-dev*` prompts (same precedent
    /// as `reviewer_cannot_merge`, mika#2248), so renaming it would break a
    /// contract for a cosmetic gain. Since mika#2617 the population is **every**
    /// red check of the head, required by branch protection or not; what changed
    /// to say so is the `detail`, which now names the checks instead of counting
    /// "required" ones.
    #[serde(rename = "required_check_failed")]
    RequiredCheckFailed { failing_checks: Vec<CheckInfo> },
    /// At least one check is still running, and the gate waits rather than
    /// delegating the wait to GitHub (mika#2617 U2).
    ///
    /// This replaces [`MergeGateResult::AutoMergeEnabled`] on the tool path. The
    /// old arm armed `gh pr merge --auto`, and GitHub then merges as soon as the
    /// **required** checks pass — i.e. under the definition of "green" this
    /// ticket exists to stop using. Reading every check and then arming `--auto`
    /// would have left the fix inert on exactly that path.
    ///
    /// Nothing is lost by refusing here: `ci_success_handler` already re-enters
    /// the merge path on `check_suite.completed(success)` and demands a strict
    /// `AllPassed` (mika#571), so it is both the re-entry point and the only one
    /// of the two that uses our own definition of green.
    #[serde(rename = "checks_pending")]
    ChecksPending { pending_checks: Vec<CheckInfo> },
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
    /// The calling agent is the autonomous reviewer (mika#2248). Merging under
    /// its credentials stamps `mergedBy` with the login that posted the review —
    /// the PR would close on its own approval, with no handoff. Blocked
    /// regardless of PR state: this is an authority verdict, not a gate result.
    #[serde(rename = "reviewer_cannot_merge")]
    ReviewerCannotMerge {
        agent_id: String,
        dispatcher_agent: String,
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

/// The bucket vocabulary [`classify_checks`] actually decides on.
///
/// `skipping` and `neutral`-shaped buckets fall into `AllPassed` by design —
/// AC1 asks for it in as many words. What this constant exists for is the
/// **fourth** case: a bucket `gh` adds tomorrow would be born non-blocking, a
/// fail-open on a merge gate. The policy is deliberately unchanged (refusing on
/// an unknown bucket would break merging on another `gh` version, i.e. trade a
/// red `main` for a stopped loop); only its silence is.
pub(crate) const KNOWN_CHECK_BUCKETS: &[&str] =
    &["pass", "fail", "pending", "skipping", "cancel", "neutral"];

/// `(name, bucket)` of every check whose bucket is outside [`KNOWN_CHECK_BUCKETS`].
///
/// Pure, so the WARN its caller emits is testable without a subprocess. Returns
/// an empty vec on the nominal path — no line is emitted per evaluation
/// (doctrine mika#2131).
pub(crate) fn unknown_buckets(checks: &[GhCheck]) -> Vec<(&str, &str)> {
    checks
        .iter()
        .filter(|c| !KNOWN_CHECK_BUCKETS.contains(&c.bucket.as_str()))
        .map(|c| (c.name.as_str(), c.bucket.as_str()))
        .collect()
}

// ---------------------------------------------------------------------------
// Gate decision (pure function — mika#2617, plan R12)
// ---------------------------------------------------------------------------

/// What the check list alone says the gate must do.
///
/// **Extracted rather than tested through `execute`**, because `execute` offers
/// no seam: it calls `run_gh_checks` and `run_gh_merge` directly, on free
/// functions, with nothing substitutable. Opening one would move the confidence
/// from a compiler-checked shape onto a subprocess fixture; the house motif is
/// the opposite — pull the decision out and let `execute` be the plumbing
/// (`decide_content_net` mika#2270, `classify_wrapper_activity` mika#2184,
/// `screen_target_purges` mika#2619).
///
/// `execute` matches this **exhaustively, with no `_ =>` arm**, so a variant
/// added by phase B or C cannot fall into a silent default.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MergeGateDecision {
    /// At least one check is red or cancelled — refuse, naming them.
    ChecksFailed { failing: Vec<CheckInfo> },
    /// No check is red and at least one is still running — refuse, naming them.
    ///
    /// This is the arm mika#2617 U2 turned from a merge into a refusal: it used
    /// to arm `gh pr merge --auto`, which hands the decision back to branch
    /// protection — the very definition of "green" this ticket exists to stop
    /// using. See [`BlockReason::ChecksPending`].
    ChecksPending { pending: Vec<CheckInfo> },
    /// Every check passed, was skipped, or there is none — merge now.
    AllChecksPassed,
}

/// Decide the gate's check arm from the check list and nothing else.
pub(crate) fn decide_merge_gate(checks: &[GhCheck]) -> MergeGateDecision {
    match classify_checks(checks) {
        CheckClassification::HasFailures => MergeGateDecision::ChecksFailed {
            failing: collect_checks(checks, &["fail", "cancel"], true),
        },
        CheckClassification::HasPending => MergeGateDecision::ChecksPending {
            pending: collect_checks(checks, &["pending"], false),
        },
        CheckClassification::AllPassed => MergeGateDecision::AllChecksPassed,
    }
}

/// Project the checks of the given buckets into the wire shape.
///
/// `keep_link` is false for pending checks: a run still in flight has no useful
/// log to point at, and the historical `auto_merge_enabled` payload carried
/// `link: None` there — keeping that shape means the field's absence stays
/// "nothing to link", never "we stopped reading it".
fn collect_checks(checks: &[GhCheck], buckets: &[&str], keep_link: bool) -> Vec<CheckInfo> {
    checks
        .iter()
        .filter(|c| buckets.contains(&c.bucket.as_str()))
        .map(|c| CheckInfo {
            name: c.name.clone(),
            state: c.state.clone(),
            link: if keep_link { c.link.clone() } else { None },
        })
        .collect()
}

/// Human-readable list of check names, for the `detail` an operator reads.
///
/// AC3 asks that the refusal **name** the check. The count it replaced
/// (`"{} required check(s) failed"`) carried neither the name nor, after
/// mika#2617, a true adjective.
pub(crate) fn describe_checks(checks: &[CheckInfo]) -> String {
    checks
        .iter()
        .map(|c| format!("{} ({})", c.name, c.state))
        .collect::<Vec<_>>()
        .join(", ")
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

/// Run `gh pr checks <number> --repo <repo> --json name,state,bucket,link`
/// and return the parsed check list.
pub(crate) async fn run_gh_checks(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<GhCheck>, String> {
    parse_gh_checks(&run_gh_checks_raw(pr_number, repo, token).await?)
}

/// The single site that composes the `gh pr checks` argv (mika#2617).
///
/// A pure function rather than an inline `vec!` because the one assertion that
/// can go red on the pre-mika#2617 tree is **structural**: `classify_checks`
/// never saw the flag, so a fixture carrying "a red check in the list" was
/// already refused before this ticket, and a behavioural test of the gate's
/// decision is a positive control rather than proof of the fix (plan R9). The
/// argv is the only surface where "which checks does the gate even look at?" is
/// decidable without a network call.
pub(crate) fn gh_checks_args(pr_number: u64, repo: &str) -> Vec<String> {
    vec![
        "pr".to_string(),
        "checks".to_string(),
        pr_number.to_string(),
        "--repo".to_string(),
        repo.to_string(),
        "--json".to_string(),
        "name,state,bucket,link".to_string(),
    ]
}

/// The network half of [`run_gh_checks`]: raw stdout of
/// `gh pr checks <n> --repo <r> --json name,state,bucket,link`.
///
/// Split out by mika#2455 so its guard can substitute the subprocess in a test
/// while keeping the production parser and classifier on the path. This stays
/// the single reader of "which check blocks" (D6) — four consumers descend from
/// it (`pr_merge_with_gate::execute`, `server::verdict_handler`,
/// `server::ci_success_handler`, and the mika#2455 CI↔verdict guard), and all
/// four decide through `classify_checks`, so they inherit whatever this call
/// returns without a line of their own changing.
///
/// **`--required` was removed on 2026-10-02 (mika#2617), and the doc-comment it
/// replaces asserted the opposite** — that the flag "is what makes this the
/// single reader of which check blocks". Measured on 2026-10-01: PR #2614 was
/// merged by the engine (`mergedBy: mika-platform-dev`, 02:54:59Z) with
/// `Egress Uniqueness Lint` and `Egress Manifest Lint` already in FAILURE on its
/// head `8ccaabc8`. Neither is a required check of `main`'s ruleset, so for a
/// gate reading `--required` that PR was green — while the rest of the house,
/// the orchestrator recipe included, never merges over a red check of any kind.
/// `main` went red (`41a4a20e`) and every open PR inherited it (mika#2616, p0).
///
/// The gate is therefore now **stricter than branch protection**, deliberately:
/// a lint added tomorrow is blocking by default instead of being born invisible
/// to the engine. Do not restore the flag believing you are repairing a
/// regression — the divergence between two definitions of "green" is the defect,
/// and this side is the one that fails closed.
pub(crate) async fn run_gh_checks_raw(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<String, String> {
    let args = gh_checks_args(pr_number, repo);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();

    run_gh_subprocess(&argv, token).await
}

/// Prefix of the error [`parse_gh_checks`] returns when `gh` answered but its
/// output is not exploitable JSON.
///
/// **Wire format, and mika#2455 is why it is a named constant.** That guard
/// must tell `gh_failed` (the call did not go through) from `unparseable`
/// (it did, and said something we cannot read) — two different remedies —
/// and `run_gh_checks` flattens both into a `String`. Matching a literal at
/// the reading site would be the substring-on-a-rendered-message the house
/// forbids (mika#2179); sharing the constant makes the discrimination a
/// contract between two sites rather than a coincidence. Pinned by
/// `mika2455_the_parse_error_prefix_is_a_wire_format`.
pub(crate) const GH_CHECKS_PARSE_ERROR_PREFIX: &str = "Failed to parse gh pr checks output";

/// Parse the stdout of `gh pr checks --json name,state,bucket,link`.
///
/// Extracted from [`run_gh_checks`] by mika#2455 so the network half can be
/// substituted in a test while the parsing stays the production one — a second
/// parser written for the test would attest the test's parser, not this one.
///
/// Empty output or `[]` means **no check at all** — not "no required check",
/// which is what this said until mika#2617 removed the `--required` flag from
/// [`gh_checks_args`]. `classify_checks` reads it as `AllPassed`, a deliberate
/// pre-existing semantics carried here unchanged: a repository that runs no CI
/// is not a repository whose CI is red.
pub(crate) fn parse_gh_checks(output: &str) -> Result<Vec<GhCheck>, String> {
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }

    serde_json::from_str::<Vec<GhCheck>>(trimmed)
        .map_err(|e| format!("{GH_CHECKS_PARSE_ERROR_PREFIX}: {e}"))
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

/// How many times, and how far apart, an accepted update-branch is re-read
/// before its landing is called unobserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LandingBudget {
    pub(crate) attempts: u32,
    pub(crate) delay: Duration,
}

/// Three re-reads, two seconds apart — six seconds at worst.
///
/// **This value is a choice, not a measurement, and saying so is part of it.**
/// The real landing latency of a `202 update-branch` on this repository has
/// never been observed. What makes a short budget SAFE is not the number but
/// the benignity of its error: an update that lands after the budget expires is
/// read as `AcceptedNotLanded`, which **releases the claim**, and the next
/// arrival re-measures `is_behind_main` and finds the PR no longer behind. No
/// double merge commit follows, and nothing is lost but one turn.
///
/// What makes it ACCEPTABLE IN COST is measured: of the three sites that reach
/// [`remediate_behind_main`], only the tool path is inside a bounded envelope
/// (`PrMergeWithGateTool::timeout_secs` = 60 s), so six seconds is a tenth of
/// the tightest budget — and it is paid *only* when nothing lands, i.e. in the
/// case that costs six hours today. The two webhook handlers have no enclosing
/// timeout and absorb it.
///
/// Revise it on the distribution the `accepted_not_landed` trace produces
/// (mika#2252), never on intuition. If that token ever carries nominal traffic,
/// read first whether those PRs eventually land — then the value is too low —
/// or never do, in which case the GitHub job is genuinely failing and the cause
/// is upstream of this constant.
const UPDATE_LANDING_BUDGET: LandingBudget = LandingBudget {
    attempts: 3,
    delay: Duration::from_secs(2),
};

/// What the post-acceptance re-read of the PR's base established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LandingObservation {
    /// The base moved, and this is the SHA that was READ off the PR.
    Landed { observed_base_sha: String },
    /// The budget expired without the base being seen to move. Says nothing
    /// about *why* — a slow async job and a failed one look identical here, and
    /// pretending otherwise is the class of claim mika#2252 exists to remove.
    NotObserved,
}

/// Re-read the PR's base until it moves, or until the budget runs out.
///
/// The effect is INJECTED (`read_base`) rather than called directly, and that
/// is what makes AC3/AC4 assertable without a forge: the decision is a pure
/// loop over a closure, the network lives at the call site. Same idiom, same
/// reason, as `server::deadline_verdict`'s `poster`.
///
/// Two properties are contractual:
///
/// **The wait PRECEDES the first read.** The endpoint is asynchronous by
/// contract, so reading immediately would produce a systematic false negative —
/// the exact remedy the ticket's first draft prescribed and that this shape
/// replaces. The loop returns on the first success, so a fast landing pays one
/// delay and the full budget is only ever spent when nothing lands.
///
/// **An unreadable base is never a landing.** An `Err`, a SHA equal to
/// `before_base_sha`, **or an empty SHA** all count as "not yet" and continue.
/// The empty-SHA term is load-bearing rather than defensive:
/// `PrPreflight::base_ref_oid` falls back to the empty string when GitHub does
/// not render the field (pinned by `preflight_base_ref_oid_defaults_to_empty`),
/// so without it `"" != before_base_sha` would be true and an absent field
/// would read as a landing — the very defect being closed, one layer down.
pub(crate) async fn observe_update_branch_landing<F, Fut>(
    before_base_sha: &str,
    budget: LandingBudget,
    mut read_base: F,
) -> LandingObservation
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    for _ in 0..budget.attempts {
        tokio::time::sleep(budget.delay).await;

        if let Ok(observed) = read_base().await
            && !observed.is_empty()
            && observed != before_base_sha
        {
            return LandingObservation::Landed {
                observed_base_sha: observed,
            };
        }
    }

    LandingObservation::NotObserved
}

/// Outcome of one `update-branch` call against a PR.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UpdateBranchOutcome {
    /// GitHub took the request — and that is the whole of what it says.
    ///
    /// The endpoint answers `202 Accepted` and performs the merge
    /// asynchronously, so a zero exit proves the request was accepted, never
    /// that a commit exists (mika#2252). The name is the fix: this variant used
    /// to be called `Updated` and to claim "a new commit now sits on the PR
    /// head", which no line of code had read. Whether the update landed is
    /// established downstream by re-reading the PR's base — see
    /// [`observe_update_branch_landing`] — and only that reading produces
    /// [`BehindMainRemediation::Updated`].
    Accepted,
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
    /// The re-read OBSERVED the PR's base move: the update landed, and a fresh
    /// CI run is expected on the new head.
    ///
    /// The SHA is the one that was read off the PR (mika#2252), never
    /// `BehindMainInfo::current_main_sha`, which is the SHA that was *aimed
    /// at*. The two differ whenever `main` moves again between the update and
    /// the re-read, and publishing the second under the name of the first is
    /// the defect this variant's payload exists to close.
    Updated { observed_base_sha: String },
    /// GitHub accepted the update and the re-read never saw the base move
    /// inside the landing budget (mika#2252).
    ///
    /// Three facts about this state, each load-bearing. It is **not** a
    /// failure: the request was taken, and the async job may still be running
    /// or may have failed — the engine cannot tell which, and says so rather
    /// than guessing. It **releases the anti-thrash claim**, because a claim
    /// spent on an update nobody observed landing would otherwise strand the
    /// PR for the six-hour TTL. And it is **not** [`Self::Updated`], so no
    /// caller can read "repaired" off an outcome nothing measured.
    AcceptedNotLanded,
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
/// Which outcomes give the claim back is decided by [`releases_claim`], whose
/// doc-comment carries the per-variant reasoning.
///
/// The cost of releasing, stated: a persistently failing update (a 403) is
/// retried once per webhook rather than once per `main` SHA. That is a bounded
/// number of extra `gh` calls on a PR that is already blocked and already
/// notifying the operator — cheaper than a PR nobody comes back to.
fn release_update_branch_attempt(pr_number: u64, repo: &str, target_main_sha: &str) {
    UPDATE_ATTEMPTS.remove(&format!("{repo}#{pr_number}@{target_main_sha}"));
}

/// Does this outcome give its anti-thrash claim back?
///
/// **An exhaustive `match` with no `_` arm, and that is the point of extracting
/// it.** The release used to be an inline `matches!` in
/// [`remediate_behind_main`] — the one site in this chain the compiler could
/// not hold, since `matches!` keeps compiling when a variant appears (the shape
/// mika#1940 had to name in writing). A new variant now fails to compile here,
/// where the decision is, instead of silently defaulting to "keep the claim"
/// six hours at a time. Being pure also makes it assertable without a forge,
/// which is the discipline the rest of this module already follows.
///
/// Two outcomes release, for two different reasons:
///
/// - `Failed` — the attempt never reached GitHub, so it made no commit and is
///   not the thrash the cap exists to stop.
/// - `AcceptedNotLanded` — GitHub took the request and nothing was seen to
///   land. Keeping the claim would strand the PR until the six-hour TTL or the
///   next move of `main`, which is the state mika#2238 exists to end and
///   mika#2252 found still reachable through the accepted-but-not-landed door.
///
/// The rest keep it. `Updated` is the case the cap is *for*; `Conflict` is a
/// stable fact about the PR that re-asking cannot change; `AlreadyAttempted`
/// never took a claim of its own; `NotBehind`, `Contradiction` and
/// `BaseNotMain` describe a PR the repair must not be re-aimed at.
pub(crate) fn releases_claim(remediation: &BehindMainRemediation) -> bool {
    match remediation {
        BehindMainRemediation::Failed(_) | BehindMainRemediation::AcceptedNotLanded => true,
        BehindMainRemediation::Updated { .. }
        | BehindMainRemediation::AlreadyAttempted
        | BehindMainRemediation::NotBehind
        | BehindMainRemediation::Conflict(_)
        | BehindMainRemediation::Contradiction(_)
        | BehindMainRemediation::BaseNotMain(_) => false,
    }
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
/// **`Accepted` means accepted, and the name is now the whole claim.** The
/// endpoint answers `202 Accepted` and performs the merge asynchronously, so a
/// zero exit proves GitHub took the request, never that a commit exists. This
/// function therefore establishes nothing about landing, and since mika#2252 it
/// no longer pretends to: whether the update landed is read off the PR's base
/// by [`observe_update_branch_landing`], and only that reading yields
/// [`BehindMainRemediation::Updated`].
///
/// The failure this used to leave open — GitHub accepts, the async job fails,
/// no commit, no CI, no webhook — is no longer bounded by the six-hour TTL. An
/// acceptance whose landing is not observed becomes
/// [`BehindMainRemediation::AcceptedNotLanded`], which releases the claim, so
/// the next arrival re-measures instead of waiting for `main` to move.
pub(crate) async fn attempt_update_branch(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> UpdateBranchOutcome {
    let endpoint = format!("repos/{repo}/pulls/{pr_number}/update-branch");
    let args = vec!["api", "--method", "PUT", &endpoint];

    match run_gh_subprocess(&args, token).await {
        Ok(_) => UpdateBranchOutcome::Accepted,
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
        // A `202` says the request was taken. Whether a commit exists is read
        // off the PR, never inferred from the exit code (mika#2252).
        UpdateBranchOutcome::Accepted => {
            let observation =
                observe_update_branch_landing(&info.pr_base_sha, UPDATE_LANDING_BUDGET, || async {
                    run_gh_pr_view(pr_number, repo, token)
                        .await
                        .map(|preflight| preflight.base_ref_oid)
                        .map_err(|e| e.message)
                })
                .await;

            match observation {
                LandingObservation::Landed { observed_base_sha } => {
                    BehindMainRemediation::Updated { observed_base_sha }
                }
                LandingObservation::NotObserved => BehindMainRemediation::AcceptedNotLanded,
            }
        }
        UpdateBranchOutcome::AlreadyUpToDate => {
            reconcile_already_up_to_date(pr_number, repo, token)
                .await
                .unwrap_or_else(BehindMainRemediation::Contradiction)
        }
        UpdateBranchOutcome::Conflict(detail) => BehindMainRemediation::Conflict(detail),
        UpdateBranchOutcome::Failed(detail) => BehindMainRemediation::Failed(detail),
    };

    // Which outcomes give the claim back — and why each one does or does not —
    // lives in `releases_claim`, an exhaustive match the compiler holds.
    if releases_claim(&remediation) {
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

/// The `outcome` token and optional `detail` of one behind-main decision.
///
/// **The tokens are a WIRE FORMAT.** They land in `$MIKA_SPIRIT_LOG_FILE` and
/// [`log_behind_main_remediation`]'s doc-comment publishes one of them as an
/// operator predicate, so two spellings of one outcome would split a population
/// without saying so. Pure, and an exhaustive `match` with no `_` arm, so the
/// set is pinnable by test and a new variant is a compile error rather than a
/// silent `"unknown"`.
///
/// `"updated"` is deliberately UNCHANGED by mika#2252: renaming it would break
/// a published reading for no gain, and after that fix it is *truer* than it
/// was — it now designates a landing that was observed, where it used to
/// designate an acceptance nobody had read. `"accepted_not_landed"` is the
/// added token, and it is what makes the two populations countable apart.
fn behind_main_trace_fields(remediation: &BehindMainRemediation) -> (&'static str, Option<&str>) {
    match remediation {
        BehindMainRemediation::Updated { .. } => ("updated", None),
        BehindMainRemediation::AcceptedNotLanded => ("accepted_not_landed", None),
        BehindMainRemediation::AlreadyAttempted => ("already_attempted", None),
        BehindMainRemediation::NotBehind => ("not_behind", None),
        BehindMainRemediation::Conflict(d) => ("conflict", Some(d.as_str())),
        BehindMainRemediation::Failed(d) => ("failed", Some(d.as_str())),
        BehindMainRemediation::Contradiction(d) => ("contradiction", Some(d.as_str())),
        BehindMainRemediation::BaseNotMain(d) => ("base_not_main", Some(d.as_str())),
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
    let (outcome, detail) = behind_main_trace_fields(remediation);

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
        // `new_main_sha` is the SHA that was READ off the PR, never
        // `info.current_main_sha`, which is the SHA that was aimed at. This
        // field is DATA, serialized to the monitor and to the LLM: filling it
        // from the target would declare a base nothing had measured, which is
        // the defect mika#2252 closes.
        BehindMainRemediation::Updated { observed_base_sha } => {
            Some(MergeGateResult::BranchUpdated {
                pr_base_sha: info.pr_base_sha.clone(),
                new_main_sha: observed_base_sha.clone(),
            })
        }
        // Blocked[behind_main] rather than a seventh `MergeGateResult` variant,
        // for three reasons in order of weight. It is the FACT: the base did not
        // move, so the PR is still behind. The precedent is the `AlreadyAttempted`
        // arm immediately below — same shape, same reason, different cause. And
        // the three embedded prompts enumerate **six** variants while instructing
        // the agent to branch on them exhaustively, so a seventh would reopen T2
        // of mika#2238 — an agent meeting a variant outside its list has no
        // defined move, and the observed behaviour is a silent stop.
        BehindMainRemediation::AcceptedNotLanded => Some(MergeGateResult::Blocked {
            reason: BlockReason::BehindMain {
                pr_base_sha: info.pr_base_sha.clone(),
                current_main_sha: info.current_main_sha.clone(),
            },
            failing_checks: vec![],
            detail: format!(
                "PR is behind main (base: {}, main HEAD: {}) — GitHub accepted an automatic \
                 branch update, but the PR's base had not moved when it was re-read, so the \
                 update is not confirmed to have landed. A fresh attempt is open: the next \
                 arrival re-measures rather than waiting.",
                info.pr_base_sha, info.current_main_sha
            ),
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
        BehindMainRemediation::AcceptedNotLanded => Some(format!(
            "the PR is behind main (base: {base}, main HEAD: {head}). GitHub accepted an \
             automatic branch update, but the PR's base had not moved when it was re-read, so \
             the update is NOT confirmed to have landed (mika#2252). It may still be running \
             on GitHub's side, or it may have failed — the engine cannot tell which, and does \
             not guess. A fresh attempt is open for the next arrival. Do NOT merge this PR and \
             do NOT call `pr_merge_with_gate` for it. Do NOT rebase by hand. End the turn.\n\n"
        )),
        BehindMainRemediation::Updated { observed_base_sha } => Some(format!(
            "the PR was behind main (base: {base}, main HEAD: {head}). An automatic branch \
             update was accepted and the PR's base was then read at {observed_base_sha} \
             (mika#2238, mika#2252), which moves the PR's head to a new commit. Do NOT merge \
             this PR in this turn and do NOT call \
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
            // mika#1964 measured this site OUT of the population, and that is a
            // divergence from the ticket's inventory, declared rather than absorbed.
            // The inventory reads this text as "serialized into the MergeGateResult
            // JSON served as content" — true of `classify_credential_scope_error`
            // (reached through `disposition_for_remediation`), false here. This
            // function composes a webhook PRE-DIGEST, consumed only by
            // `ci_success_handler` and `verdict_handler`, i.e. by mika-dev / mika-qa
            // on operator tier: a family tenant receives no GitHub webhook and can
            // never read it. It also has no `ToolContext`, and neither of its two
            // callers builds a `ToolOutput`, so there is no channel to route to — and
            // on operator tier `dispatch_substrate_diagnostic` folds the diagnostic
            // back into the content anyway, so a conversion would change no served
            // byte. Should a family-reachable caller ever appear, this becomes a real
            // leak and the annotation must go.
            //
            // The marker sits on the LAST comment line on purpose: an annotation
            // exempts a short window, so a marker buried at the top of a long comment
            // block would leave the literal outside it (mika#1964 §3.3 decision 3).
            // substrate-ok: webhook pre-digest, operator tier only — see above.
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

/// Run `gh pr merge <number> --repo <repo> --<method> [--delete-branch]`
/// and return stdout on success or stderr on failure.
///
/// **There is no `auto` parameter, and its absence is the mechanism (mika#2617
/// U2).** `--auto` asks GitHub to merge as soon as the **required** checks
/// pass — a second, laxer definition of "green" living inside the gate that
/// U1 had just taught to read every check. Removing the parameter makes the
/// flag inexpressible rather than discouraged: the compiler forced the three
/// remaining call sites, where a lexical scan over a positional `bool` would
/// have been fragile (doctrine mika#1991, *build the incapacity, do not promise
/// the restraint*).
///
/// The wait it replaces is not lost. `ci_success_handler` re-enters the merge
/// path on `check_suite.completed(success)` and requires a strict `AllPassed`
/// (mika#571), so it was already the redundant half — and the only one of the
/// two that reads our own definition of green.
pub(crate) async fn run_gh_merge(
    pr_number: u64,
    repo: &str,
    merge_method: &str,
    delete_branch: bool,
    token: &str,
) -> Result<String, String> {
    let pr_str = pr_number.to_string();
    let method_flag = format!("--{merge_method}");
    let mut args = vec!["pr", "merge", &pr_str, "--repo", repo, &method_flag];

    if delete_branch {
        args.push("--delete-branch");
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
    // substrate-ok: an env() argument handed to a child process — it never enters a
    // tool-result `content`, so no tier reads it.
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

/// Persist the supervisor's `$.claude_pilot.pr_url` when the gate refuses on
/// pending checks (mika#1211, moved here by mika#2617 U2).
///
/// **It is the same need under a new arm, not a new mechanism.** The write
/// existed to neutralise the orphan reaper's `pr_url IS NULL` predicate (#871)
/// for a supervisor whose child is waiting on a PR that is open and not merged.
/// That state is now reached through `checks_pending` instead of
/// `auto_merge_enabled`; dropping the call with the arm would have let a
/// perfectly healthy supervisor be flipped to `failed` 600 s later.
///
/// Mirror of `dispatcher::try_extract_callback_metadata`
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
async fn write_pending_pr_url_to_supervisor(ctx: &ToolContext<'_>, pr_url: &str) {
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
            "pr_merge_with_gate: wrote pr_url to supervisor metadata on checks_pending"
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

    // Since mika#1964 this `detail` IS the operator channel — `emit_gate_result`
    // moves it out of the model-visible `content` and routes it, replacing it with
    // CREDENTIAL_SCOPE_NEUTRAL_DETAIL. The remedy it names stays whole, which is
    // mika#1616's contract: the agent must report a real cause rather than paraphrase
    // an exit code. Only its reader changed.
    Some(MergeGateResult::GateError {
        kind: GateErrorKind::CredentialScope {
            repo: repo.to_string(),
        },
        // substrate-diagnostic: routed by `emit_gate_result` — see above.
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

    // ─────────────────────────────────────────────────────────────────────
    // mika#2617 — la porte lit TOUS les checks (U1)
    // ─────────────────────────────────────────────────────────────────────

    /// **Le seul test de ce ticket qui rougit sur l'arbre d'avant (plan R9).**
    ///
    /// `classify_checks` ne voit pas le drapeau : une fixture « un check rouge
    /// dans la liste » était déjà refusée avant mika#2617, donc aucun test de
    /// comportement ne peut attester le correctif. La preuve est l'argv.
    #[test]
    fn mika2617_gh_checks_args_carries_no_required_flag() {
        let args = gh_checks_args(2614, "senara-solutions/mika");
        assert!(
            !args.iter().any(|a| a == "--required"),
            "mika#2617 — `gh pr checks` ne doit plus restreindre aux checks requis : \
             la protection de branche de `main` ne couvrait ni `Egress Uniqueness Lint` \
             ni `Egress Manifest Lint`, et la PR #2614 a été mergée par le moteur avec \
             les deux en FAILURE. argv = {args:?}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2617 U4 — zéro liste d'exemptions (AC3)
    // ─────────────────────────────────────────────────────────────────────

    /// Les quatre fichiers où vit la décision de merge du moteur.
    const MERGE_GATE_SOURCES: &[&str] = &[
        "tools/pr_merge_with_gate.rs",
        "server/verdict_handler.rs",
        "server/ci_success_handler.rs",
        "server/merge_ready_handler.rs",
    ];

    /// Mots qui, accolés à `CHECK` dans le nom d'une constante, désignent une
    /// liste d'exemptions. **Livrée non vide : c'est le vocabulaire cherché,
    /// pas une allowlist.**
    const CHECK_WAIVER_WORDS: &[&str] = &["EXEMPT", "ADVISORY", "IGNORE", "SKIP", "ALLOW", "WAIVE"];

    /// Allowlist du scan d'exemptions — **livrée vide, et épinglée vide**.
    const ALLOWED_CHECK_EXEMPTION_SITES: &[&str] = &[];

    /// Allowlist du scan de lecture de label — **livrée vide, et épinglée vide**.
    const ALLOWED_LABEL_READING_SITES: &[&str] = &[];

    fn merge_gate_production_half(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(rel);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("mika#2617 — {} illisible : {e}", path.display()));
        crate::source_scan::strip_comment_lines(crate::source_scan::production_half(&content))
    }

    /// Le prédicat du scan d'exemptions, isolé pour que son contrôle de bonne
    /// foi puisse l'exercer sur une fixture plutôt que sur l'arbre.
    fn declares_a_check_exemption_list(src: &str) -> Vec<String> {
        let mut hits = Vec::new();
        for line in src.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("const ")
                .or_else(|| trimmed.strip_prefix("static "))
                .or_else(|| trimmed.strip_prefix("pub const "))
                .or_else(|| trimmed.strip_prefix("pub static "))
                .or_else(|| trimmed.strip_prefix("pub(crate) const "))
                .or_else(|| trimmed.strip_prefix("pub(crate) static "))
            else {
                continue;
            };
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if ident.is_empty() || !ident.contains("CHECK") {
                continue;
            }
            if CHECK_WAIVER_WORDS.iter().any(|w| ident.contains(w)) {
                hits.push(ident);
            }
        }
        hits
    }

    /// **AC3 — aucune constante ne peut exempter un check rouge.**
    ///
    /// Le prédicat porte sur le **nom**, quel que soit le type : une
    /// `EXEMPT_CHECKS: &[&str]` et une `EXEMPT_CHECKS: HashSet<_>` disent la
    /// même chose. Aucun test de comportement ne peut voir cette classe — une
    /// liste ajoutée demain ne rend aucune décision fausse le jour où elle est
    /// écrite, elle rend la porte plus laxiste en silence.
    #[test]
    fn mika2617_no_check_exemption_list_exists() {
        let mut offenders = Vec::new();
        for rel in MERGE_GATE_SOURCES {
            for ident in declares_a_check_exemption_list(&merge_gate_production_half(rel)) {
                if !ALLOWED_CHECK_EXEMPTION_SITES.contains(&ident.as_str()) {
                    offenders.push(format!("{rel}: {ident}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "mika#2617 AC3 — une liste d'exemptions de checks a été déclarée : {offenders:?}\n\n\
             RÉSOLUTION : la retirer. Un check qu'on ne veut pas voir bloquer est un check \
             à retirer de la CI, pas à exempter ici — une liste nommée recrée très \
             exactement la divergence entre deux définitions du vert que ce ticket ferme \
             (doctrine mika#2201 : on route le site, on n'allowliste pas)."
        );
    }

    /// Contrôle de bonne foi du scan ci-dessus : il mord sur une fixture.
    ///
    /// Sans lui, un prédicat devenu inopérant (renommage, changement de forme
    /// de déclaration) se lirait exactement comme un arbre propre — classe
    /// mika#2103 / mika#2205.
    #[test]
    fn mika2617_the_exemption_scan_reddens_on_a_fixture() {
        let fixture = "\
pub(crate) const ADVISORY_CHECKS: &[&str] = &[\"Docker Build\"];
const KNOWN_CHECK_BUCKETS: &[&str] = &[\"pass\"];
const DISPATCH_SKIP_REASONS: &[&str] = &[\"x\"];
";
        let hits = declares_a_check_exemption_list(fixture);
        assert_eq!(
            hits,
            vec!["ADVISORY_CHECKS".to_string()],
            "le prédicat doit voir la liste d'exemptions et ignorer ses deux voisines \
             légitimes (une sans mot de renonciation, une sans CHECK)"
        );
    }

    /// Les deux allowlists sont livrées vides et le restent.
    ///
    /// Sans ce test, « on déclare, on n'allowliste pas » ne vivrait que dans un
    /// doc-comment — et un doc-comment n'a jamais fait rougir une CI. Le jour
    /// où quelqu'un ajoute une entrée, il doit d'abord supprimer ce test, ce
    /// qui est un geste visible en revue (mika#2323).
    #[test]
    fn mika2617_the_exemption_allowlists_are_empty() {
        assert!(
            ALLOWED_CHECK_EXEMPTION_SITES.is_empty(),
            "ALLOWED_CHECK_EXEMPTION_SITES est livrée vide et doit le rester"
        );
        assert!(
            ALLOWED_LABEL_READING_SITES.is_empty(),
            "ALLOWED_LABEL_READING_SITES est livrée vide et doit le rester"
        );
    }

    /// Les formes par lesquelles une étiquette **GitHub** entre dans le moteur.
    ///
    /// Délibérément distinctes de `Task.label` / `DEFERRED_DISPATCH_LABEL`, qui
    /// sont des étiquettes **internes** et peuplent légitimement ces fichiers.
    const GITHUB_LABEL_READS: &[&str] = &[".labels", "\"labels\"", "--add-label", "--remove-label"];

    fn reads_a_github_label(src: &str) -> Vec<String> {
        src.lines()
            .filter(|l| GITHUB_LABEL_READS.iter().any(|n| l.contains(n)))
            .map(|l| l.trim().to_string())
            .collect()
    }

    /// **AC3 — aucun label de PR n'entre dans la décision de merge.**
    ///
    /// Un label est écrivable à la main par n'importe qui ayant accès au dépôt.
    /// Le brancher ici transformerait un geste d'interface en **autorisation de
    /// merge**, ce que mika#2248 a déjà dû refuser pour le signal
    /// `merge-ready` : la porte franchirait alors sur un fait que la personne
    /// qui le pose n'a pas qualifié.
    #[test]
    fn mika2617_no_label_is_read_in_the_merge_decision() {
        let mut offenders = Vec::new();
        for rel in MERGE_GATE_SOURCES {
            for line in reads_a_github_label(&merge_gate_production_half(rel)) {
                if !ALLOWED_LABEL_READING_SITES.contains(&line.as_str()) {
                    offenders.push(format!("{rel}: {line}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "mika#2617 AC3 — un label GitHub est lu dans la chaîne de décision de merge : \
             {offenders:?}\n\n\
             RÉSOLUTION : le retirer. Un label est écrivable par un humain ; en faire un \
             terme de la porte transforme un geste d'interface en autorisation de merge \
             (refus déjà posé par mika#2248 pour `merge-ready`)."
        );
    }

    /// Contrôle de bonne foi du scan de label.
    #[test]
    fn mika2617_the_label_scan_reddens_on_a_fixture() {
        let fixture = "\
let labels = pr.labels.iter().map(|l| l.name.clone());
let task_id_label = task.label.clone();
if c.label == crate::agent::DEFERRED_DISPATCH_LABEL {}
";
        let hits = reads_a_github_label(fixture);
        assert_eq!(
            hits.len(),
            1,
            "le prédicat doit voir la lecture de `pr.labels` et ignorer les deux \
             étiquettes internes : {hits:?}"
        );
    }

    /// **AC3 — `classify_checks` refuse un check rouge QUEL QUE SOIT son nom.**
    ///
    /// Test de propriété sur les 22 noms du pont (b) ratifié par Vincent, plus
    /// les quatre que le pont laisse volontairement hors ruleset et un nom
    /// arbitraire. C'est la moitié comportementale de « zéro exemption » : un
    /// filtre sur le nom se verrait ici même sans constante déclarée.
    #[test]
    fn mika2617_classify_checks_is_blind_to_the_check_name() {
        const NAMES: &[&str] = &[
            // Les 22 du pont (b).
            "A2A Timeout Literal Lint",
            "Byte Slice Lint",
            "Canonical Token Lint",
            "CTA Primitives Lint",
            "Cwd Guard Lint",
            "Dispatch Seat Declaration Lint",
            "Egress Manifest Lint",
            "Egress No-Log Lint",
            "Egress Request Shape Lint",
            "Egress Uniqueness Lint",
            "Image Tag Immutability Lint",
            "Issue Annotation Guard Lint",
            "Landing Token Lint",
            "Loop Select Lint",
            "Pilot Push Site Lint",
            "Pilot Turn-Ceiling Label Lint",
            "Publish Verify Lint",
            "Shared Checkout Guard Lint",
            "SIGPIPE grep-q Lint",
            "Substrate Leak Lint",
            "Voice Non-Transit Lint",
            "Secret Scan",
            // Hors pont, volontairement — ils doivent bloquer tout autant.
            "Docker Build",
            "Pilot Egress Status Tap",
            "validate",
            // Et un nom que personne n'a prévu.
            "zorglub",
        ];

        for name in NAMES {
            let red = vec![GhCheck {
                name: (*name).to_string(),
                state: "FAILURE".to_string(),
                bucket: "fail".to_string(),
                link: None,
            }];
            assert_eq!(
                classify_checks(&red),
                CheckClassification::HasFailures,
                "mika#2617 AC3 — le check `{name}` rouge doit bloquer"
            );

            let green = vec![GhCheck {
                name: (*name).to_string(),
                state: "SUCCESS".to_string(),
                bucket: "pass".to_string(),
                link: None,
            }];
            assert_eq!(
                classify_checks(&green),
                CheckClassification::AllPassed,
                "contrôle positif — le check `{name}` vert ne doit pas bloquer"
            );
        }
    }

    /// **U2 — `run_gh_merge` a exactement trois sites d'appel (plan R15).**
    ///
    /// Le nombre est celui d'**après** U2 : le retrait du paramètre `auto` fait
    /// disparaître la branche `HasPending` du tool, qui ne merge plus mais
    /// refuse. L'assertion est donc autant une mesure d'U2 qu'une garde — si
    /// elle rend quatre, U2 n'a pas pris.
    #[test]
    fn mika2617_run_gh_merge_has_exactly_three_call_sites() {
        let mut sites = Vec::new();
        for rel in [
            "tools/pr_merge_with_gate.rs",
            "server/verdict_handler.rs",
            "server/merge_ready_handler.rs",
            "server/ci_success_handler.rs",
            "server/ci_failure_handler.rs",
        ] {
            for line in merge_gate_production_half(rel).lines() {
                if line.contains("run_gh_merge(") && !line.contains("fn run_gh_merge(") {
                    sites.push(format!("{rel}: {}", line.trim()));
                }
            }
        }
        assert_eq!(
            sites.len(),
            3,
            "mika#2617 R15 — attendu trois sites d'appel de `run_gh_merge` après U2 \
             (le tool sur sa seule branche `AllPassed`, `verdict_handler`, \
             `merge_ready_handler`). Trouvés : {sites:#?}"
        );

        // Anti-vacuité : un scan qui ne regarderait plus rien rendrait aussi
        // zéro, et `assert_eq!(0, 3)` est le seul garde-fou qu'un renommage de
        // fichier laisserait debout. On exige les trois fichiers attendus.
        for expected in [
            "tools/pr_merge_with_gate.rs",
            "server/verdict_handler.rs",
            "server/merge_ready_handler.rs",
        ] {
            assert!(
                sites.iter().any(|s| s.starts_with(expected)),
                "mika#2617 — aucun appel trouvé dans {expected} : le scan vise un \
                 chemin mort et ne vérifie rien. Trouvés : {sites:#?}"
            );
        }
    }

    /// Le `link` est la dépendance silencieuse de la relance (phase B, U3) et du
    /// contexte de réparation de `ci_failure_handler`. Le perdre ne casserait
    /// rien de visible ici et rendrait les deux inertes.
    #[test]
    fn mika2617_gh_checks_args_still_requests_the_link_field() {
        let args = gh_checks_args(2614, "senara-solutions/mika");
        let json_value = args
            .iter()
            .position(|a| a == "--json")
            .and_then(|i| args.get(i + 1))
            .expect("l'argv porte --json <champs>");
        for field in ["name", "state", "bucket", "link"] {
            assert!(
                json_value.split(',').any(|f| f == field),
                "mika#2617 — le champ `{field}` a disparu du --json : {json_value}"
            );
        }
        assert_eq!(args[0], "pr");
        assert_eq!(args[1], "checks");
        assert_eq!(args[2], "2614");
        assert!(args.iter().any(|a| a == "senara-solutions/mika"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2617 — les cas de la décision pure (plan § 4, périmètre phase A)
    //
    // Ils vivent ici et non sous `tests/eval/` : `decide_merge_gate` est
    // `pub(crate)`, et élargir la visibilité d'une fonction de production pour
    // l'emplacement d'un fichier de test est le mauvais arbitrage — précédent
    // écrit de mika#2532 pour `spawn_long_running_exec`.
    // ─────────────────────────────────────────────────────────────────────

    fn check(name: &str, bucket: &str, state: &str) -> GhCheck {
        GhCheck {
            name: name.to_string(),
            state: state.to_string(),
            bucket: bucket.to_string(),
            link: Some(format!(
                "https://github.com/o/r/actions/runs/1/job/2#{name}"
            )),
        }
    }

    /// **T1 — un check rouge hors des six requis d'avant le pont bloque, et le
    /// `detail` le NOMME.**
    ///
    /// Rejeu du défaut fondateur : `Egress Uniqueness Lint` et `Egress Manifest
    /// Lint` étaient en FAILURE sur la tête `8ccaabc8` de la PR #2614, et la
    /// porte a mergé.
    #[test]
    fn mika2617_t1_a_non_required_red_check_blocks_and_is_named() {
        let checks = vec![
            check("Check", "pass", "SUCCESS"),
            check("Egress Uniqueness Lint", "fail", "FAILURE"),
            check("Egress Manifest Lint", "fail", "FAILURE"),
        ];
        let MergeGateDecision::ChecksFailed { failing } = decide_merge_gate(&checks) else {
            panic!("un check rouge doit fermer la porte");
        };
        assert_eq!(failing.len(), 2);

        let detail = describe_checks(&failing);
        assert!(detail.contains("Egress Uniqueness Lint"), "{detail}");
        assert!(detail.contains("Egress Manifest Lint"), "{detail}");
        assert!(
            !detail.contains("required"),
            "le `detail` ne doit plus qualifier les checks de « requis » : {detail}"
        );
        assert!(
            failing[0].link.is_some(),
            "le `link` d'un check rouge est conservé — la relance (phase B) en dérive le run_id"
        );
    }

    /// **T2 — contrôle positif : tous verts, la porte s'ouvre.**
    ///
    /// Sans lui, « la porte décide » est indistinguable de « la porte bloque
    /// tout » — et ce second état casserait la boucle en entier avec toutes les
    /// autres assertions au vert (leçon mika#2277).
    #[test]
    fn mika2617_t2_all_green_opens_the_gate() {
        let checks = vec![
            check("Check", "pass", "SUCCESS"),
            check("Egress Uniqueness Lint", "pass", "SUCCESS"),
        ];
        assert_eq!(
            decide_merge_gate(&checks),
            MergeGateDecision::AllChecksPassed
        );
        assert_eq!(decide_merge_gate(&[]), MergeGateDecision::AllChecksPassed);
    }

    /// **T3 — contrôle positif : `skipping` et `neutral` comptent comme passés
    /// (AC1).**
    #[test]
    fn mika2617_t3_skipped_and_neutral_count_as_passed() {
        let checks = vec![
            check("Docker Build (self-hosted canary)", "skipping", "SKIPPED"),
            check("Pilot Egress Status Tap", "neutral", "NEUTRAL"),
            check("Check", "pass", "SUCCESS"),
        ];
        assert_eq!(
            decide_merge_gate(&checks),
            MergeGateDecision::AllChecksPassed
        );
    }

    /// **T4 — un check en attente REFUSE, au lieu d'armer `--auto`.**
    #[test]
    fn mika2617_t4_a_pending_check_refuses_instead_of_arming_auto_merge() {
        let checks = vec![
            check("Check", "pass", "SUCCESS"),
            check("Docker Build", "pending", "IN_PROGRESS"),
        ];
        let MergeGateDecision::ChecksPending { pending } = decide_merge_gate(&checks) else {
            panic!("un check en attente doit fermer la porte (mika#2617 U2)");
        };
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].name, "Docker Build");
        assert!(
            pending[0].link.is_none(),
            "un run en vol n'a pas de log utile à pointer — même forme que la \
             charge `auto_merge_enabled` qu'il remplace"
        );
        assert!(describe_checks(&pending).contains("Docker Build"));
    }

    /// Un rouge l'emporte sur une attente : la porte rapporte ce qui est cassé,
    /// pas ce qui tourne encore.
    #[test]
    fn mika2617_a_red_check_wins_over_a_pending_one() {
        let checks = vec![
            check("Docker Build", "pending", "IN_PROGRESS"),
            check("Egress Manifest Lint", "fail", "FAILURE"),
        ];
        let MergeGateDecision::ChecksFailed { failing } = decide_merge_gate(&checks) else {
            panic!("le rouge prime");
        };
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].name, "Egress Manifest Lint");
    }

    /// Le refus `checks_pending` porte son nom de fil, et il est lisible par un
    /// prompt qui branche sur `reason.reason`.
    #[test]
    fn mika2617_checks_pending_serializes_under_its_wire_name() {
        let result = MergeGateResult::Blocked {
            reason: BlockReason::ChecksPending {
                pending_checks: vec![CheckInfo {
                    name: "Docker Build".to_string(),
                    state: "IN_PROGRESS".to_string(),
                    link: None,
                }],
            },
            failing_checks: vec![],
            detail: "1 check(s) still running".to_string(),
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
        assert_eq!(json["action"], "blocked");
        assert_eq!(json["reason"]["reason"], "checks_pending");
        assert_eq!(json["reason"]["pending_checks"][0]["name"], "Docker Build");
    }

    /// Un bucket hors du vocabulaire connu est **rapporté** et reste non
    /// bloquant (plan R6) : refuser sur l'inconnu casserait le merge sur une
    /// autre version de `gh`, c'est-à-dire échangerait un `main` rouge contre
    /// une boucle arrêtée.
    #[test]
    fn mika2617_an_unknown_bucket_is_reported_and_stays_non_blocking() {
        let checks = vec![
            check("Check", "pass", "SUCCESS"),
            check("Zorglub", "quantum", "WEIRD"),
        ];
        assert_eq!(
            unknown_buckets(&checks),
            vec![("Zorglub", "quantum")],
            "le bucket inconnu doit être recensé pour la ligne WARN"
        );
        assert_eq!(
            decide_merge_gate(&checks),
            MergeGateDecision::AllChecksPassed,
            "la politique ne change pas — seul son silence change"
        );
        assert!(
            unknown_buckets(&[check("Check", "pass", "SUCCESS")]).is_empty(),
            "contrôle négatif — rien n'est émis sur le chemin nominal (mika#2131)"
        );
    }

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

    /// A base SHA the re-read OBSERVED — deliberately equal to neither
    /// `pr_base_sha` nor `current_main_sha` of [`behind_info`].
    ///
    /// That third value is what makes "measured" testable at all: with an
    /// observed SHA equal to the target, every assertion below would pass on
    /// the pre-mika#2252 code, which published the target under the name of a
    /// reading nothing had made.
    const OBSERVED_SHA: &str = "0b5e4ved0b5e4ved0b5e4ved0b5e4ved0b5e4ved";

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
        let disposition = disposition_for_remediation(
            &BehindMainRemediation::Updated {
                observed_base_sha: OBSERVED_SHA.to_string(),
            },
            REPO,
            &info,
        );

        assert!(
            disposition.is_some(),
            "an updated branch must terminate the turn, never continue into the merge gate"
        );
        assert_eq!(
            disposition,
            Some(MergeGateResult::BranchUpdated {
                pr_base_sha: info.pr_base_sha.clone(),
                // Since mika#2252 this field carries the SHA that was READ, not
                // the one that was aimed at — see `mika2252_branch_updated_*`.
                new_main_sha: OBSERVED_SHA.to_string(),
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
            BehindMainRemediation::Updated {
                observed_base_sha: OBSERVED_SHA.to_string(),
            },
            BehindMainRemediation::AcceptedNotLanded,
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

    // -----------------------------------------------------------------------
    // mika#2252 — `Accepted` is not `Updated`: measuring the landing
    // -----------------------------------------------------------------------

    /// A budget with no wall-clock cost. `attempts` is what the tests vary;
    /// the delay only has to be *some* duration for the loop to be exercised.
    fn instant_budget(attempts: u32) -> LandingBudget {
        LandingBudget {
            attempts,
            delay: Duration::ZERO,
        }
    }

    const BEFORE_SHA: &str = "abc1234deadbeef";

    #[tokio::test]
    async fn mika2252_a_base_that_never_moves_is_not_observed() {
        // The founding case. GitHub answered `202`, the async job produced no
        // commit, and the base still reads what it read before. Nothing has
        // landed, and the engine must say so rather than publish a repair.
        let observation = observe_update_branch_landing(BEFORE_SHA, instant_budget(3), || async {
            Ok(BEFORE_SHA.to_string())
        })
        .await;

        assert_eq!(observation, LandingObservation::NotObserved);
    }

    #[tokio::test]
    async fn mika2252_the_loop_stops_on_the_first_observed_move() {
        // The loop returns on success: exactly 2 calls out of a budget of 3.
        // A third would mean a fast landing still pays the whole budget, i.e.
        // 6 s added to every repaired PR instead of only to the unrepaired ones.
        let calls = std::cell::Cell::new(0u32);

        let observation = observe_update_branch_landing(BEFORE_SHA, instant_budget(3), || {
            let n = calls.get() + 1;
            calls.set(n);
            let answer = if n >= 2 {
                Ok(OBSERVED_SHA.to_string())
            } else {
                Ok(BEFORE_SHA.to_string())
            };
            async move { answer }
        })
        .await;

        assert_eq!(
            observation,
            LandingObservation::Landed {
                observed_base_sha: OBSERVED_SHA.to_string(),
            },
            "the SHA carried must be the one that was READ, not the one aimed at"
        );
        assert_eq!(
            calls.get(),
            2,
            "the loop must stop on the first observed move — a fast landing \
             must not pay the whole budget"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mika2252_the_wait_precedes_the_first_read() {
        // R3, and it needs a CLOCK to be measurable at all.
        //
        // The call count cannot attest this — verified by mutation: deleting
        // the `sleep` leaves every other test in this section green, because a
        // read-then-wait loop reaches the same SHA at the same iteration. What
        // separates the two orders is *when* the first read happens, so that is
        // what this asserts.
        //
        // It matters because the endpoint is asynchronous by contract: a read
        // issued at t=0 sees the pre-update base, and on a budget of 1 that is
        // a systematic false negative on a PR that lands perfectly well — the
        // remedy the ticket's first draft prescribed and that this shape exists
        // to replace.
        //
        // `start_paused` gives tokio a virtual clock it auto-advances while
        // idle, so the two seconds below are free and deterministic.
        let budget = LandingBudget {
            attempts: 1,
            delay: Duration::from_secs(2),
        };

        let started = tokio::time::Instant::now();
        let first_read_at = std::cell::Cell::new(None::<Duration>);

        let observation = observe_update_branch_landing(BEFORE_SHA, budget, || {
            if first_read_at.get().is_none() {
                first_read_at.set(Some(started.elapsed()));
            }
            async { Ok(OBSERVED_SHA.to_string()) }
        })
        .await;

        assert_eq!(
            observation,
            LandingObservation::Landed {
                observed_base_sha: OBSERVED_SHA.to_string(),
            }
        );
        assert_eq!(
            first_read_at.get(),
            Some(budget.delay),
            "the first re-read must happen AFTER one delay, never at t=0 — \
             reading immediately would answer `NotObserved` on every PR whose \
             update lands normally"
        );
    }

    #[tokio::test]
    async fn mika2252_an_unreadable_base_is_never_a_landing() {
        // R4, first negative control. `gh` failing is not evidence that the
        // update landed; it is the absence of evidence either way. Kept
        // SEPARATE from its two siblings on purpose: a conjunction of fail-safe
        // terms is not proven by neutralising them together — a predicate
        // reading only one of the three would pass a combined test.
        let observation = observe_update_branch_landing(BEFORE_SHA, instant_budget(3), || async {
            Err("gh exit code 1: Not Found (HTTP 404)".to_string())
        })
        .await;

        assert_eq!(observation, LandingObservation::NotObserved);
    }

    #[tokio::test]
    async fn mika2252_an_empty_base_is_never_a_landing() {
        // R4, second negative control — and the load-bearing one.
        // `PrPreflight::base_ref_oid` falls back to the empty string when
        // GitHub does not render the field (pinned by
        // `preflight_base_ref_oid_defaults_to_empty`), so that population
        // exists. Without the emptiness term, `"" != before_base_sha` is true
        // and an ABSENT FIELD reads as a landing — the defect this ticket
        // closes, reproduced one layer down.
        let observation = observe_update_branch_landing(BEFORE_SHA, instant_budget(3), || async {
            Ok(String::new())
        })
        .await;

        assert_eq!(
            observation,
            LandingObservation::NotObserved,
            "an unrendered base field must never be read as a landing"
        );
    }

    #[test]
    fn mika2252_an_unlanded_acceptance_blocks_behind_main_and_gives_its_claim_back() {
        // AC3, both halves. The disposition half alone is not enough: a test
        // asserting only `Blocked[behind_main]` would stay green with R6
        // forgotten and D2 — the six-hour stall — wide open.
        let info = behind_info();

        let disposition =
            disposition_for_remediation(&BehindMainRemediation::AcceptedNotLanded, REPO, &info)
                .expect("an unlanded acceptance must terminate the turn, never merge");

        match &disposition {
            MergeGateResult::Blocked { reason, detail, .. } => {
                assert_eq!(
                    *reason,
                    BlockReason::BehindMain {
                        pr_base_sha: info.pr_base_sha.clone(),
                        current_main_sha: info.current_main_sha.clone(),
                    },
                    "the PR IS still behind — that is the fact, not a fallback"
                );
                assert!(
                    detail.contains("not confirmed to have landed"),
                    "the detail must say what was not established: {detail}"
                );
            }
            other => panic!("expected blocked[behind_main], got {other:?}"),
        }

        assert!(
            !matches!(disposition, MergeGateResult::BranchUpdated { .. }),
            "the loop must never read `repaired` off a landing nobody observed"
        );

        // The claim half, exercised against the real process-global ledger.
        assert!(
            releases_claim(&BehindMainRemediation::AcceptedNotLanded),
            "an acceptance whose landing was not observed must give its claim \
             back — keeping it strands the PR for the six-hour TTL (D2)"
        );

        let sha = "mika2252-not-landed-unique-sha-91af3c";
        assert!(claim_update_branch_attempt(2252, REPO, sha));
        assert!(!claim_update_branch_attempt(2252, REPO, sha));
        release_update_branch_attempt(2252, REPO, sha);
        assert!(
            claim_update_branch_attempt(2252, REPO, sha),
            "a released claim must be re-takeable — otherwise the next arrival \
             cannot re-measure and the stall is back"
        );
    }

    #[test]
    fn mika2252_branch_updated_carries_the_observed_sha_not_the_target() {
        // AC2, and the test that separates "measured" from "declared". It only
        // discriminates because the observed SHA differs from BOTH shas of
        // `behind_info()` — with `observed == current_main_sha` this assertion
        // would pass on the pre-fix code, which published `info.current_main_sha`
        // under the name of a reading nothing had made.
        let info = behind_info();
        assert_ne!(
            OBSERVED_SHA, info.current_main_sha,
            "the fixture must distinguish the read SHA from the aimed-at one"
        );

        let disposition = disposition_for_remediation(
            &BehindMainRemediation::Updated {
                observed_base_sha: OBSERVED_SHA.to_string(),
            },
            REPO,
            &info,
        )
        .expect("an updated branch must terminate the turn");

        match disposition {
            MergeGateResult::BranchUpdated { new_main_sha, .. } => {
                assert_eq!(
                    new_main_sha, OBSERVED_SHA,
                    "`new_main_sha` is serialized to the monitor and to the LLM: \
                     it must be the base that was READ, never the one aimed at"
                );
                assert_ne!(
                    new_main_sha, info.current_main_sha,
                    "publishing the target would declare a base nothing measured"
                );
            }
            other => panic!("expected branch_updated, got {other:?}"),
        }
    }

    #[test]
    fn mika2252_an_observed_landing_keeps_its_claim() {
        // AC4. The anti-thrash cap of mika#2238 R4 is untouched for the
        // nominal case: an update that landed is exactly what the cap exists
        // for, and releasing there would re-open the thrash loop this fix is
        // built on top of.
        assert!(!releases_claim(&BehindMainRemediation::Updated {
            observed_base_sha: OBSERVED_SHA.to_string(),
        }));

        let sha = "mika2252-landed-keeps-claim-unique-sha-5d7b2e";
        assert!(claim_update_branch_attempt(2252, REPO, sha));
        assert!(
            !claim_update_branch_attempt(2252, REPO, sha),
            "an observed landing must NOT give its claim back"
        );
    }

    #[test]
    fn mika2252_only_failure_and_unlanded_acceptance_release_the_claim() {
        // The other four outcomes keep it, each for a reason written on
        // `releases_claim`. Enumerated rather than spot-checked so that a
        // future variant routed to `true` by reflex reddens here.
        for keeps in [
            BehindMainRemediation::Updated {
                observed_base_sha: OBSERVED_SHA.to_string(),
            },
            BehindMainRemediation::AlreadyAttempted,
            BehindMainRemediation::NotBehind,
            BehindMainRemediation::Conflict("merge conflict".to_string()),
            BehindMainRemediation::Contradiction("still behind".to_string()),
            BehindMainRemediation::BaseNotMain("release/1.4".to_string()),
        ] {
            assert!(
                !releases_claim(&keeps),
                "{keeps:?} must keep its claim — see `releases_claim` for why"
            );
        }
        assert!(releases_claim(&BehindMainRemediation::Failed(
            "HTTP 403".to_string()
        )));
    }

    #[test]
    fn mika2252_merge_gate_result_still_has_exactly_six_variants() {
        // AC8 — the guard that keeps this fix from re-opening T2 of mika#2238.
        //
        // The three embedded prompts (`self-dev`, `self-dev-webhook-ci`,
        // `self-dev-webhook-qa`) state that `pr_merge_with_gate` returns **six**
        // typed variants and instruct the agent to branch on them exhaustively.
        // An agent meeting a seventh has no defined move, and the observed
        // behaviour is a silent stop — the mika#2236 shape.
        //
        // The REAL protection is the exhaustive `match` below, which fails to
        // compile on a new variant and names the site. The count assertion is
        // the anti-vacuity term: without it a scan that stopped looking at
        // anything would read exactly like a clean tree.
        fn action_token(result: &MergeGateResult) -> &'static str {
            match result {
                MergeGateResult::Merged => "merged",
                MergeGateResult::AutoMergeEnabled { .. } => "auto_merge_enabled",
                MergeGateResult::Blocked { .. } => "blocked",
                MergeGateResult::AlreadyMerged => "already_merged",
                MergeGateResult::GateError { .. } => "gate_errored",
                MergeGateResult::BranchUpdated { .. } => "branch_updated",
            }
        }

        let one_of_each = [
            MergeGateResult::Merged,
            MergeGateResult::AutoMergeEnabled {
                pending_checks: vec![],
            },
            MergeGateResult::Blocked {
                reason: BlockReason::MergeConflict,
                failing_checks: vec![],
                detail: String::new(),
            },
            MergeGateResult::AlreadyMerged,
            MergeGateResult::GateError {
                kind: GateErrorKind::Unknown,
                detail: String::new(),
            },
            MergeGateResult::BranchUpdated {
                pr_base_sha: String::new(),
                new_main_sha: String::new(),
            },
        ];

        let tokens: Vec<&'static str> = one_of_each.iter().map(action_token).collect();
        assert_eq!(
            tokens.len(),
            6,
            "mika#2252 AC8 — `MergeGateResult` must still carry exactly six \
             variants. If a seventh was added, the three `self-dev*` prompts \
             saying \"six typed variants … branch exhaustively\" must be \
             updated IN THE SAME COMMIT; do not relax this assertion. Tokens \
             seen: {tokens:?}"
        );

        // The tokens are the serde tags an agent actually branches on.
        for (result, token) in one_of_each.iter().zip(&tokens) {
            let json = serde_json::to_value(result).expect("MergeGateResult serializes");
            assert_eq!(json["action"], *token, "serde tag drifted for {token}");
        }
    }

    #[test]
    fn mika2252_the_behind_main_outcome_tokens_are_a_wire_format() {
        // AC6. These land in `$MIKA_SPIRIT_LOG_FILE` and one of them is
        // published as an operator predicate, so a rename is a break to DATE in
        // `CLAUDE.md` — never a test quietly brought into line.
        let cases: [(BehindMainRemediation, &str); 8] = [
            (
                BehindMainRemediation::Updated {
                    observed_base_sha: OBSERVED_SHA.to_string(),
                },
                "updated",
            ),
            (
                BehindMainRemediation::AcceptedNotLanded,
                "accepted_not_landed",
            ),
            (BehindMainRemediation::AlreadyAttempted, "already_attempted"),
            (BehindMainRemediation::NotBehind, "not_behind"),
            (BehindMainRemediation::Conflict("c".to_string()), "conflict"),
            (BehindMainRemediation::Failed("f".to_string()), "failed"),
            (
                BehindMainRemediation::Contradiction("x".to_string()),
                "contradiction",
            ),
            (
                BehindMainRemediation::BaseNotMain("release/1.4".to_string()),
                "base_not_main",
            ),
        ];

        for (remediation, expected) in &cases {
            let (token, _) = behind_main_trace_fields(remediation);
            assert_eq!(
                token, *expected,
                "outcome token drifted for {remediation:?} — this is a wire \
                 format read by operators; restore it or date the break"
            );
        }

        // Anti-vacuity: the set must stay complete AND distinct. A duplicated
        // token would merge two populations while every line above passed.
        let mut tokens: Vec<&str> = cases.iter().map(|(_, t)| *t).collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(
            tokens.len(),
            8,
            "the eight behind-main outcomes must have eight distinct tokens"
        );
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
        // mika#1964 — `harness.ctx()` is operator tier, so the diagnostic is folded
        // back into `content` and the variable is still readable here. The family
        // half is `mika1964_missing_token_does_not_leak_on_family_tier` below.
        assert!(result.content.contains("MIKA_GITHUB_TOKEN"));
        assert!(
            result
                .content
                .starts_with("Merging pull requests is not available"),
            "the neutral fallback must come first: {}",
            result.content
        );
    }

    /// mika#1964 V3 — the leak M1 made reachable is closed.
    ///
    /// `pr_merge_with_gate` is registered by `default_tools()`, so it sits in every
    /// agent's tool array whatever its tier, and `FAMILY_IDENTITY` declares no
    /// `[tools].disabled` block. The old message was therefore served to a family
    /// tenant — which is why the ticket's "currently latent because
    /// FAMILY_AGENT_SKILL_ALLOWLIST excludes them" premise was false: that allowlist
    /// gates skills, never builtin tools.
    #[tokio::test]
    async fn mika1964_missing_token_does_not_leak_on_family_tier() {
        let harness = TestHarness::new();
        let mut ctx = harness.ctx();
        ctx.tier = mika_common::home::AgentTier::Family;

        let result = PrMergeWithGateTool
            .execute(
                json!({"pr_number": 42, "repo": "senara-solutions/mika"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        for token in ["MIKA_GITHUB_TOKEN", "GitHub App", "config.toml"] {
            assert!(
                !result.content.contains(token),
                "family tier leaked {token:?}: {}",
                result.content
            );
        }
        // The fallback must still say the merge did not happen — a content that says
        // nothing makes the model invent a cause (mika#1783 risk 2).
        assert!(result.content.contains("Nothing was merged"));

        let events = harness
            .db
            .get_audit_events("test-session")
            .await
            .expect("get_audit_events");
        let routed: Vec<_> = events
            .iter()
            .filter(|e| {
                e.tool_name == "substrate_unavailable" && e.target_key == "pr_merge_with_gate"
            })
            .collect();
        assert_eq!(
            routed.len(),
            1,
            "expected one routed diagnostic: {events:?}"
        );
        assert!(
            routed[0]
                .after_value
                .as_deref()
                .unwrap_or("")
                .contains("MIKA_GITHUB_TOKEN"),
            "the operator detail did not reach the diagnostic channel"
        );
    }

    /// The credential-scope result as `classify_credential_scope_error` builds it
    /// from the gh 403 of the mika#1616 root cause.
    fn credential_scope_result() -> MergeGateResult {
        classify_credential_scope_error(
            "gh exit code 1: HTTP 403: Resource not accessible by integration",
            "senara-solutions/mika-cloud",
        )
        .expect("a 403 classifies as credential scope")
    }

    fn substrate_rows(
        events: &[crate::evidence::audit::AuditEvent],
    ) -> Vec<&crate::evidence::audit::AuditEvent> {
        events
            .iter()
            .filter(|e| {
                e.tool_name == "substrate_unavailable" && e.target_key == "pr_merge_with_gate"
            })
            .collect()
    }

    /// mika#1964 — the flagship site of the sweep, reached directly.
    ///
    /// `emit_gate_result`'s CredentialScope arm is only reachable through `.execute()`
    /// with real gh 403 output, and this file has no gh harness — so the two tests
    /// above stop at the missing-token guard and never touch it. This calls it in
    /// place, on both sealed tiers.
    ///
    /// The shape is pinned as well as the leak: a successful, parseable JSON whose
    /// `action` the model branches on, never an `is_error` result with prose after
    /// the JSON (which `tool_execution/dispatch.rs` would record as a failed call).
    #[tokio::test]
    async fn mika1964_credential_scope_does_not_leak_on_sealed_tiers() {
        for tier in [
            mika_common::home::AgentTier::Family,
            mika_common::home::AgentTier::Champion,
        ] {
            let harness = TestHarness::new();
            let mut ctx = harness.ctx();
            ctx.tier = tier;

            let out = emit_gate_result(credential_scope_result(), &ctx)
                .await
                .unwrap();

            assert!(!out.is_error, "{tier:?}: a refusal is not a failed call");
            let json: serde_json::Value = serde_json::from_str(&out.content)
                .unwrap_or_else(|e| panic!("{tier:?}: content is not JSON ({e}): {}", out.content));
            assert_eq!(json["action"], "gate_errored");
            assert_eq!(json["kind"]["kind"], "credential_scope");
            assert_eq!(json["detail"], CREDENTIAL_SCOPE_NEUTRAL_DETAIL);
            for token in [
                "GitHub App",
                "PAT",
                "`repo` scope",
                "Resource not accessible",
            ] {
                assert!(
                    !out.content.contains(token),
                    "{tier:?} leaked {token:?}: {}",
                    out.content
                );
            }

            let events = harness
                .db
                .get_audit_events("test-session")
                .await
                .expect("get_audit_events");
            let routed = substrate_rows(&events);
            assert_eq!(routed.len(), 1, "{tier:?}: expected one routed diagnostic");
            let after = routed[0].after_value.as_deref().unwrap_or("");
            assert!(
                after.contains("GitHub App") && after.contains("senara-solutions/mika-cloud"),
                "{tier:?}: the remedy did not reach the diagnostic channel: {after}"
            );
        }
    }

    /// mika#1964 — the operator-tier half: the result is served whole, byte for byte
    /// as before the sweep, and nothing is routed to the telemetry sink.
    #[tokio::test]
    async fn mika1964_credential_scope_default_tier_keeps_the_whole_json() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        assert_eq!(ctx.tier, mika_common::home::AgentTier::Default);

        let expected = serde_json::to_string_pretty(&credential_scope_result()).unwrap();
        let out = emit_gate_result(credential_scope_result(), &ctx)
            .await
            .unwrap();

        assert!(!out.is_error);
        assert_eq!(
            out.content, expected,
            "operator tier must be byte-identical"
        );
        let json: serde_json::Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(json["action"], "gate_errored");
        assert!(json["detail"].as_str().unwrap().contains("GitHub App"));

        let events = harness
            .db
            .get_audit_events("test-session")
            .await
            .expect("get_audit_events");
        assert!(
            substrate_rows(&events).is_empty(),
            "operator tier must not write a substrate row: {events:?}"
        );
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
    // These tests exercise `write_pending_pr_url_to_supervisor` directly
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
            is_webhook_fallthrough_turn: false,
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
            deployment: mika_common::home::Deployment::Unknown,
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

    // ---- Authority gate: the reviewer is never a merge actor (mika#2248) ----

    /// `pr_merge_with_gate` appelé par l'agent relecteur : refusé avant tout
    /// appel `gh`. Le contexte n'a pas de token et l'entrée est valide — si la
    /// porte d'autorité ne tenait pas, le test verrait l'erreur de token, pas le
    /// blocage. C'est ce qui rend le refus attribuable à l'identité.
    #[tokio::test]
    async fn mika2248_le_relecteur_est_refuse_avant_tout_appel_gh() {
        let harness = TestHarness::with_agent(mika_common::forge_identity::REVIEWER_AGENT);
        let ctx = harness.ctx();
        let out = PrMergeWithGateTool
            .execute(
                json!({"pr_number": 2244, "repo": "senara-solutions/mika"}),
                &ctx,
            )
            .await
            .expect("tool must not error out");

        let parsed: serde_json::Value = serde_json::from_str(&out.content).expect("output is JSON");
        assert_eq!(parsed["action"], "blocked", "got {}", out.content);
        assert_eq!(parsed["reason"]["reason"], "reviewer_cannot_merge");
        assert_eq!(
            parsed["reason"]["agent_id"],
            mika_common::forge_identity::REVIEWER_AGENT
        );
        assert!(
            parsed["detail"]
                .as_str()
                .unwrap()
                .contains(mika_common::forge_identity::DISPATCHER_AGENT),
            "the refusal must name who does own the merge: {}",
            out.content
        );
    }

    /// Le contrôle négatif du même appel : le dispatcher n'est PAS retenu par
    /// cette porte. Sans lui, un refus inconditionnel passerait le test ci-dessus
    /// et casserait le merge autonome dans le silence.
    #[tokio::test]
    async fn mika2248_le_dispatcher_passe_la_porte_didentite() {
        let harness = TestHarness::with_agent(mika_common::forge_identity::DISPATCHER_AGENT);
        let ctx = harness.ctx();
        let out = PrMergeWithGateTool
            .execute(
                json!({"pr_number": 2244, "repo": "senara-solutions/mika"}),
                &ctx,
            )
            .await
            .expect("tool must not error out");

        // `ctx.github_token` est `None` : l'appel avance jusqu'à l'exigence de
        // token, donc au-delà de la porte d'identité. Depuis mika#1964 ce refus
        // porte son détail opérateur par le canal de diagnostic, replié dans le
        // `content` sur le tier opérateur — d'où le nom de variable comme témoin
        // qu'on a bien atteint la vérification du jeton.
        assert!(
            out.content.contains("MIKA_GITHUB_TOKEN"),
            "the dispatcher must reach the token check, not an authority refusal: {}",
            out.content
        );
        assert!(
            !out.content.contains("reviewer_cannot_merge"),
            "the dispatcher must never be refused as a reviewer: {}",
            out.content
        );
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
        write_pending_pr_url_to_supervisor(&ctx, pr_url).await;

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

        write_pending_pr_url_to_supervisor(
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

        write_pending_pr_url_to_supervisor(&ctx, "https://github.com/senara-solutions/mika/pull/9")
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

        write_pending_pr_url_to_supervisor(&ctx, "https://github.com/senara-solutions/mika/pull/9")
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
        write_pending_pr_url_to_supervisor(&ctx, pr_url).await;

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

        write_pending_pr_url_to_supervisor(&ctx, pr_url).await;
        write_pending_pr_url_to_supervisor(&ctx, pr_url).await;

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
        write_pending_pr_url_to_supervisor(&ctx, "https://github.com/senara-solutions/mika/pull/9")
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
