//! Structural handler for `pull_request_review.submitted` webhook events.
//!
//! Intercepts PR review events **before** the LLM turn and dispatches
//! deterministically based on the `VERDICT:` line in the review body.
//! This removes verdict classification from LLM improvisation — every
//! parseable verdict token maps to an engine-level action.
//!
//! Dispatch table:
//! - `pass` → merge via `pr_merge_with_gate` (existing, #524)
//! - `block[ac]` → dispatch claude-pilot with AC-fix prompt; bounded retry
//! - `block[ci]` → dispatch claude-pilot with CI-fix prompt; bounded retry
//! - `block[security]` / `block[pipeline]` → escalate to operator; NO auto-dispatch
//! - `hold[review]` → notify operator; leave task in_progress
//! - missing/unparseable → safe-default hold[review] + structured log event
//!
//! See issues #524, #889 and the compound doc at
//! `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`.

use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::LazyLock;
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::task_state::merge_metadata;
use crate::tools::pr_merge_with_gate::{
    CheckClassification, classify_checks, run_gh_checks, run_gh_merge, run_gh_subprocess,
};

use super::verdict::{
    PrReviewEvent, Verdict, parse_pr_review_event, parse_review_depth, parse_verdict,
};
use super::webhook_queue::has_active_callback_child;

/// Maximum block[ac] retries before escalation.
const BLOCK_AC_MAX_RETRIES: u32 = 3;

/// Maximum block[ci] retries before escalation.
const BLOCK_CI_MAX_RETRIES: u32 = 3;

/// PR-keyed circuit breaker threshold (mika#1563).
///
/// When this many `verdict_observed` audit_events accumulate for the same
/// PR URL within `PR_CIRCUIT_BREAKER_WINDOW_SECS`, the handler short-circuits
/// — refusing to passthrough to the LLM — and escalates to operator.
///
/// Sized to match `BLOCK_AC_MAX_RETRIES` / `BLOCK_CI_MAX_RETRIES`: when the
/// task-keyed retry counter fires correctly, this never triggers. It binds
/// only when task lookup fails (the #1556 convergence case), where the
/// per-task gate cannot bind because `find_active_task_by_pr_url` returns
/// None and `handle_block_ac` early-exits via `Passthrough`.
const PR_CIRCUIT_BREAKER_THRESHOLD: i64 = 3;

/// Sliding window for the PR-keyed circuit breaker (mika#1563).
///
/// 30 minutes covers the typical mika-qa sync cadence (#1556 saw 7 cycles
/// across ~3 hours = ~25 min/cycle) while expiring fast enough that a
/// reasonable manual fix-and-resubmit can re-engage the loop.
const PR_CIRCUIT_BREAKER_WINDOW_SECS: i64 = 30 * 60;

/// `audit_events.tool_name` used by the circuit breaker. Distinct from
/// `verdict_handled` (per-block-class success log) and `verdict_escalated`
/// (task-keyed retry-limit exceeded) so the count is unambiguously PR-keyed.
const VERDICT_OBSERVED_TOOL: &str = "verdict_observed";

/// Maximum body excerpt length for fallback AC extraction and metadata.
const BODY_EXCERPT_MAX: usize = 2000;

/// Identical-diff circuit breaker threshold: halt after this many rejections
/// with the same PR head commit SHA (mika#1563).
const IDENTICAL_DIFF_THRESHOLD: u32 = 3;

/// A recorded diff fingerprint entry stored in task metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiffFingerprint {
    sha: String,
    verdict: String,
    at: String,
}

/// Result of the structural verdict handler.
#[derive(Debug)]
pub enum VerdictAction {
    /// The handler acted on the verdict (merge initiated or error).
    /// The pre-digest text should replace the original message.
    Handled { pre_digest: String },
    /// The handler did not act — pass through to the LLM.
    /// Optional enrichment text to prepend to the original message.
    Passthrough { enrichment: Option<String> },
}

/// Attempt to handle a PR review verdict structurally before the LLM turn.
///
/// Returns `VerdictAction::Handled` when the handler acted on a recognized
/// verdict (merge, dispatch, escalation, hold). Returns `VerdictAction::Passthrough`
/// only for non-review events or unrecognized block/hold subtypes.
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_pr_review_verdict(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Parse the PR review event from formatted text
    let event = match parse_pr_review_event(text) {
        Some(e) => e,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    // 2. Parse the verdict — authoritative regardless of GH review.state (#889)
    let verdict = parse_verdict(&event.body);

    // 2b. Parse and log review depth (mika#275) — informational metadata
    let review_depth = parse_review_depth(&event.body);
    info!(
        event = "verdict_review_depth",
        pr_number = event.pr_number,
        repo = %event.repo,
        review_depth = ?review_depth,
        "parsed review depth from verdict body"
    );

    // 2c. PR-keyed circuit breaker (mika#1563).
    //
    // The task-keyed retry counter in handle_block_{ac,ci} fails to bind when
    // `find_active_task_by_pr_url` returns None — exactly what happened on
    // PR #1556 (7 syncs, 0 escalations). This breaker counts observation
    // events per PR URL, independent of task lookup. After
    // PR_CIRCUIT_BREAKER_THRESHOLD observations of any blocking verdict for
    // the same PR within PR_CIRCUIT_BREAKER_WINDOW_SECS, the handler refuses
    // to passthrough and escalates.
    //
    // Pass verdicts and hold[review] are excluded — they're either terminal
    // success or operator-only paths and shouldn't trip the breaker.
    let is_blocking_verdict = matches!(verdict, Verdict::Block(_) | Verdict::Missing { .. },);
    if is_blocking_verdict
        && let Some(action) =
            pr_circuit_breaker_check(&event, db, message_sender, session_id, trace_id).await
    {
        return action;
    }

    match verdict {
        Verdict::Pass => {
            // Pass verdicts still gate on state=approved for merge safety
            if event.state != "approved" {
                return VerdictAction::Passthrough { enrichment: None };
            }
            handle_pass_verdict(
                &event,
                db,
                github_token,
                message_sender,
                session_id,
                trace_id,
            )
            .await
        }
        Verdict::Block(reason) => match reason.to_lowercase().as_str() {
            "ac" => {
                handle_block_ac(
                    &event,
                    db,
                    github_token,
                    message_sender,
                    session_id,
                    trace_id,
                )
                .await
            }
            "ci" => {
                handle_block_ci(
                    &event,
                    db,
                    github_token,
                    message_sender,
                    session_id,
                    trace_id,
                )
                .await
            }
            "security" | "pipeline" => {
                handle_escalate(&event, db, &reason, message_sender, session_id, trace_id).await
            }
            _ => {
                warn!(
                    reason = %reason,
                    pr_number = event.pr_number,
                    repo = %event.repo,
                    "Unrecognized block[*] verdict subtype — passing through to LLM"
                );
                VerdictAction::Passthrough { enrichment: None }
            }
        },
        Verdict::Hold(reason) => match reason.to_lowercase().as_str() {
            "review" => {
                handle_hold_review(
                    &event,
                    db,
                    github_token,
                    message_sender,
                    session_id,
                    trace_id,
                )
                .await
            }
            _ => {
                warn!(
                    reason = %reason,
                    pr_number = event.pr_number,
                    repo = %event.repo,
                    "Unrecognized hold[*] verdict subtype — passing through to LLM"
                );
                VerdictAction::Passthrough { enrichment: None }
            }
        },
        Verdict::Missing { truncated } => {
            handle_missing_verdict(&event, db, truncated, message_sender, session_id, trace_id)
                .await
        }
    }
}

// ---------------------------------------------------------------------------
// Pass verdict handler (existing #524 logic, unchanged)
// ---------------------------------------------------------------------------

/// Handle a VERDICT: pass — look up task, initiate merge, update metadata.
async fn handle_pass_verdict(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();

    // Require GitHub token for merge operations
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                pr_number = event.pr_number,
                repo = %event.repo,
                "VERDICT: pass but no GitHub token available — cannot initiate merge"
            );
            return VerdictAction::Passthrough {
                enrichment: Some(
                    "[verdict_handler] VERDICT: pass received but no GitHub token configured. \
                     Manual merge required.\n\n"
                        .to_string(),
                ),
            };
        }
    };

    // Look up the task by PR URL
    let task = match db.find_active_task_by_pr_url(&pr_url).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            info!(
                pr_number = event.pr_number,
                repo = %event.repo,
                pr_url = %pr_url,
                "VERDICT: pass but no active task found for PR — passing through to LLM"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_url = %pr_url,
                "Failed to look up task by PR URL — passing through to LLM"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // Only act when task is in_progress
    if task.status != "in_progress" {
        info!(
            task_id = %task.id,
            status = %task.status,
            pr_url = %pr_url,
            "VERDICT: pass but task not in_progress (status: {}) — skipping structural merge",
            task.status
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    let task_id = task.id.clone();

    // Run CI check classification (reuse pr_merge_with_gate logic).
    // Wrap in a 60-second timeout matching PrMergeWithGateTool::timeout_secs()
    // to prevent a hanging gh subprocess from blocking the agent lock.
    let checks_future = run_gh_checks(event.pr_number, &event.repo, token);
    let checks = match tokio::time::timeout(std::time::Duration::from_secs(60), checks_future).await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            warn!(
                error = %e,
                pr_number = event.pr_number,
                "Failed to fetch CI checks for structural merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(event, &e),
            };
        }
        Err(_) => {
            warn!(
                pr_number = event.pr_number,
                "CI check fetch timed out after 60s for structural merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(event, "CI check fetch timed out after 60s"),
            };
        }
    };

    let classification = classify_checks(&checks);

    match classification {
        CheckClassification::HasFailures => {
            let failing: Vec<String> = checks
                .iter()
                .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                .map(|c| format!("  - {} ({})", c.name, c.state))
                .collect();

            info!(
                pr_number = event.pr_number,
                "VERDICT: pass but CI checks failing — passing through to LLM"
            );

            VerdictAction::Passthrough {
                enrichment: Some(format!(
                    "[verdict_handler] VERDICT: pass received but CI checks are failing:\n{}\n\
                     The structural merge handler did not act. Handle the CI failures.\n\n",
                    failing.join("\n")
                )),
            }
        }
        CheckClassification::HasPending | CheckClassification::AllPassed => {
            let is_auto = classification == CheckClassification::HasPending;

            let merge_future = run_gh_merge(
                event.pr_number,
                &event.repo,
                "squash",
                true, // delete_branch
                is_auto,
                token,
            );
            let merge_result =
                tokio::time::timeout(std::time::Duration::from_secs(60), merge_future).await;

            // Flatten timeout + Result<String, String> into a single Result
            let merge_result = match merge_result {
                Ok(inner) => inner,
                Err(_) => {
                    warn!(
                        pr_number = event.pr_number,
                        "gh pr merge timed out after 60s"
                    );
                    Err("gh pr merge timed out after 60s".to_string())
                }
            };

            match merge_result {
                Ok(_output) => {
                    let action_desc = if is_auto {
                        "auto_merge_enabled"
                    } else {
                        "merge_initiated"
                    };

                    // Update task metadata
                    if let Err(e) = update_verdict_metadata(
                        db,
                        &task_id,
                        &task.metadata,
                        action_desc,
                        event.pr_number,
                        &pr_url,
                    )
                    .await
                    {
                        warn!(error = %e, task_id = %task_id, "Failed to update task metadata after merge");
                    }

                    // Log audit event
                    if let Err(e) = db
                        .log_audit_event(
                            session_id,
                            "verdict_handled",
                            &format!("task:{task_id}"),
                            Some("in_progress"),
                            Some(action_desc),
                            Some(&format!(
                                "verdict=pass action={action_desc} pr_url={pr_url} task_id={task_id}"
                            )),
                            Some(trace_id),
                        )
                        .await
                    {
                        warn!(error = %e, "Failed to log verdict_handled audit event");
                    }

                    // Send notification
                    if let Some(sender) = message_sender {
                        let notification = if is_auto {
                            format!(
                                "PR #{} on {} — VERDICT: pass from @{}. \
                                 Auto-merge enabled (CI checks still pending). \
                                 GitHub will finalize when all checks pass.",
                                event.pr_number, event.repo, event.reviewer
                            )
                        } else {
                            format!(
                                "PR #{} on {} — VERDICT: pass from @{}. \
                                 Merge initiated (squash, delete branch).",
                                event.pr_number, event.repo, event.reviewer
                            )
                        };
                        send_notification(sender, &notification).await;
                    }

                    info!(
                        pr_number = event.pr_number,
                        repo = %event.repo,
                        action = action_desc,
                        task_id = %task_id,
                        "Structural verdict handler: {action_desc} for PR #{}",
                        event.pr_number
                    );

                    VerdictAction::Handled {
                        pre_digest: format_success_pre_digest(event, action_desc, &task_id),
                    }
                }
                Err(e) => {
                    // Check for "already merged" in the error
                    let lower = e.to_lowercase();
                    if lower.contains("already merged") {
                        info!(
                            pr_number = event.pr_number,
                            "PR already merged — structural handler acknowledging"
                        );
                        return VerdictAction::Handled {
                            pre_digest: format_already_merged_pre_digest(event, &task_id),
                        };
                    }

                    warn!(
                        error = %e,
                        pr_number = event.pr_number,
                        "Structural merge failed"
                    );
                    VerdictAction::Handled {
                        pre_digest: format_error_pre_digest(event, &e),
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block[ac] handler (#889)
// ---------------------------------------------------------------------------

/// Handle a VERDICT: block[ac] — dispatch claude-pilot with AC-fix prompt,
/// bounded by retry counter and identical-diff circuit breaker (#1563).
async fn handle_block_ac(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();

    // Look up task by PR URL
    let task = match find_task_for_verdict(db, &pr_url, event).await {
        Some(t) => t,
        None => {
            return VerdictAction::Passthrough {
                enrichment: Some(format!(
                    "[verdict_handler] VERDICT: block[ac] on {} but no active in_progress task \
                     found. Passing through to LLM.\n\n",
                    pr_url
                )),
            };
        }
    };

    let task_id = task.id.clone();

    // Check if fix is already in-flight
    if has_active_callback_child(db, &task_id).await {
        info!(
            task_id = %task_id,
            pr_number = event.pr_number,
            "block[ac] but fix already in-flight — enriching passthrough"
        );
        return VerdictAction::Handled {
            pre_digest: format!(
                "<verdict_handler>\n\
                 [GitHub] PR review ({}) on {}#{} by @{}\n\
                 VERDICT: block[ac] — but a fix is already in-flight for task {task_id}.\n\n\
                 Wait for the active callback to finish before taking further action.\n\
                 Do NOT dispatch run_claude_pilot — a session is already running.\n\
                 </verdict_handler>",
                event.state, event.repo, event.pr_number, event.reviewer
            ),
        };
    }

    // Identical-diff circuit breaker (mika#1563) — runs BEFORE generic retry counter
    if let Some(action) = check_identical_diff_circuit_breaker(
        event,
        db,
        github_token,
        &task_id,
        &task.metadata,
        &pr_url,
        "block[ac]",
        message_sender,
        session_id,
        trace_id,
    )
    .await
    {
        return action;
    }

    // Read retry counter
    let count = read_verdict_retry_count(&task.metadata, "verdict_block_ac");

    // Check retry limit
    if count >= BLOCK_AC_MAX_RETRIES {
        info!(
            task_id = %task_id,
            count = count,
            "block[ac] retry limit ({}) reached — escalating",
            BLOCK_AC_MAX_RETRIES
        );

        // Mark task blocked
        if let Err(e) = db.update_task_status(&task_id, "blocked").await {
            warn!(error = %e, task_id = %task_id, "Failed to mark task blocked after ac retry limit");
        }

        // Update metadata
        if let Err(e) = update_verdict_block_metadata(
            db,
            &task_id,
            &task.metadata,
            "verdict_block_ac",
            count,
            &pr_url,
            &event.review_url,
            &truncate_body(&event.body),
        )
        .await
        {
            warn!(error = %e, task_id = %task_id, "Failed to update block[ac] escalation metadata");
        }

        // Log audit event
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "verdict_escalated",
                &format!("task:{task_id}"),
                Some("in_progress"),
                Some("blocked"),
                Some(&format!(
                    "verdict=block[ac] action=escalated_loop_limit count={count} pr_url={pr_url}"
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log verdict_escalated audit event");
        }

        // Notify operator
        if let Some(sender) = message_sender {
            send_notification(
                sender,
                &format!(
                    "PR #{} on {} — block[ac] retry limit ({}) reached from @{}. \
                     Task {} marked blocked. Operator review required.",
                    event.pr_number, event.repo, BLOCK_AC_MAX_RETRIES, event.reviewer, task_id
                ),
            )
            .await;
        }

        return VerdictAction::Handled {
            pre_digest: format_block_ac_limit_pre_digest(event, &task_id),
        };
    }

    // Extract AC list (with fallback)
    let ac_extraction = extract_ac_list_or_fallback(&event.body);
    let ac_summary = ac_extraction.summary();
    let is_fallback = ac_extraction.is_fallback();

    if is_fallback {
        warn!(
            pr_number = event.pr_number,
            repo = %event.repo,
            "verdict_ac_extraction_fallback: AC extraction yielded zero structured matches; \
             using body excerpt as fallback"
        );
    }

    let new_count = count + 1;

    // Update metadata with new count
    if let Err(e) = update_verdict_block_metadata(
        db,
        &task_id,
        &task.metadata,
        "verdict_block_ac",
        new_count,
        &pr_url,
        &event.review_url,
        &truncate_body(&event.body),
    )
    .await
    {
        warn!(error = %e, task_id = %task_id, "Failed to update block[ac] dispatch metadata");
    }

    // Log audit event
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "verdict_handled",
            &format!("task:{task_id}"),
            Some("in_progress"),
            Some("ac_fix_dispatched"),
            Some(&format!(
                "verdict=block[ac] action=ac_fix_dispatched count={new_count} pr_url={pr_url} \
                 fallback={is_fallback}"
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log verdict_handled audit event for block[ac]");
    }

    // Notify
    if let Some(sender) = message_sender {
        send_notification(
            sender,
            &format!(
                "PR #{} on {} — VERDICT: block[ac] from @{}. \
                 AC-fix dispatch preparing (attempt {new_count}/{BLOCK_AC_MAX_RETRIES}). Task: {}",
                event.pr_number, event.repo, event.reviewer, task_id
            ),
        )
        .await;
    }

    info!(
        pr_number = event.pr_number,
        repo = %event.repo,
        task_id = %task_id,
        count = new_count,
        fallback = is_fallback,
        "Structural verdict handler: block[ac] — AC-fix dispatch preparing"
    );

    VerdictAction::Handled {
        pre_digest: format_block_ac_pre_digest(event, &ac_summary, &task_id, new_count),
    }
}

// ---------------------------------------------------------------------------
// Block[ci] handler (#889)
// ---------------------------------------------------------------------------

/// Handle a VERDICT: block[ci] — dispatch claude-pilot with CI-fix prompt,
/// bounded by retry counter and identical-diff circuit breaker (#1563).
/// Same structure as block[ac].
async fn handle_block_ci(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();

    let task = match find_task_for_verdict(db, &pr_url, event).await {
        Some(t) => t,
        None => {
            return VerdictAction::Passthrough {
                enrichment: Some(format!(
                    "[verdict_handler] VERDICT: block[ci] on {} but no active in_progress task \
                     found. Passing through to LLM.\n\n",
                    pr_url
                )),
            };
        }
    };

    let task_id = task.id.clone();

    if has_active_callback_child(db, &task_id).await {
        info!(
            task_id = %task_id,
            pr_number = event.pr_number,
            "block[ci] but fix already in-flight — enriching passthrough"
        );
        return VerdictAction::Handled {
            pre_digest: format!(
                "<verdict_handler>\n\
                 [GitHub] PR review ({}) on {}#{} by @{}\n\
                 VERDICT: block[ci] — but a fix is already in-flight for task {task_id}.\n\n\
                 Wait for the active callback to finish before taking further action.\n\
                 Do NOT dispatch run_claude_pilot — a session is already running.\n\
                 </verdict_handler>",
                event.state, event.repo, event.pr_number, event.reviewer
            ),
        };
    }

    // Identical-diff circuit breaker (mika#1563) — runs BEFORE generic retry counter
    if let Some(action) = check_identical_diff_circuit_breaker(
        event,
        db,
        github_token,
        &task_id,
        &task.metadata,
        &pr_url,
        "block[ci]",
        message_sender,
        session_id,
        trace_id,
    )
    .await
    {
        return action;
    }

    let count = read_verdict_retry_count(&task.metadata, "verdict_block_ci");

    if count >= BLOCK_CI_MAX_RETRIES {
        info!(
            task_id = %task_id,
            count = count,
            "block[ci] retry limit ({}) reached — escalating",
            BLOCK_CI_MAX_RETRIES
        );

        if let Err(e) = db.update_task_status(&task_id, "blocked").await {
            warn!(error = %e, task_id = %task_id, "Failed to mark task blocked after ci retry limit");
        }

        if let Err(e) = update_verdict_block_metadata(
            db,
            &task_id,
            &task.metadata,
            "verdict_block_ci",
            count,
            &pr_url,
            &event.review_url,
            &truncate_body(&event.body),
        )
        .await
        {
            warn!(error = %e, task_id = %task_id, "Failed to update block[ci] escalation metadata");
        }

        if let Err(e) = db
            .log_audit_event(
                session_id,
                "verdict_escalated",
                &format!("task:{task_id}"),
                Some("in_progress"),
                Some("blocked"),
                Some(&format!(
                    "verdict=block[ci] action=escalated_loop_limit count={count} pr_url={pr_url}"
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log verdict_escalated audit event for block[ci]");
        }

        if let Some(sender) = message_sender {
            send_notification(
                sender,
                &format!(
                    "PR #{} on {} — block[ci] retry limit ({}) reached from @{}. \
                     Task {} marked blocked. Operator review required.",
                    event.pr_number, event.repo, BLOCK_CI_MAX_RETRIES, event.reviewer, task_id
                ),
            )
            .await;
        }

        return VerdictAction::Handled {
            pre_digest: format_block_ci_limit_pre_digest(event, &task_id),
        };
    }

    let new_count = count + 1;

    if let Err(e) = update_verdict_block_metadata(
        db,
        &task_id,
        &task.metadata,
        "verdict_block_ci",
        new_count,
        &pr_url,
        &event.review_url,
        &truncate_body(&event.body),
    )
    .await
    {
        warn!(error = %e, task_id = %task_id, "Failed to update block[ci] dispatch metadata");
    }

    if let Err(e) = db
        .log_audit_event(
            session_id,
            "verdict_handled",
            &format!("task:{task_id}"),
            Some("in_progress"),
            Some("ci_fix_dispatched"),
            Some(&format!(
                "verdict=block[ci] action=ci_fix_dispatched count={new_count} pr_url={pr_url}"
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log verdict_handled audit event for block[ci]");
    }

    if let Some(sender) = message_sender {
        send_notification(
            sender,
            &format!(
                "PR #{} on {} — VERDICT: block[ci] from @{}. \
                 CI-fix dispatch preparing (attempt {new_count}/{BLOCK_CI_MAX_RETRIES}). Task: {}",
                event.pr_number, event.repo, event.reviewer, task_id
            ),
        )
        .await;
    }

    info!(
        pr_number = event.pr_number,
        repo = %event.repo,
        task_id = %task_id,
        count = new_count,
        "Structural verdict handler: block[ci] — CI-fix dispatch preparing"
    );

    VerdictAction::Handled {
        pre_digest: format_block_ci_pre_digest(
            event,
            &truncate_body(&event.body),
            &task_id,
            new_count,
        ),
    }
}

// ---------------------------------------------------------------------------
// Escalation handler (block[security] / block[pipeline]) (#889)
// ---------------------------------------------------------------------------

/// Handle block[security] or block[pipeline] — mark task blocked, notify operator.
/// These are operator-attention-by-design; NO auto-dispatch.
async fn handle_escalate(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    reason: &str,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();

    let task = match find_task_for_verdict(db, &pr_url, event).await {
        Some(t) => t,
        None => {
            return VerdictAction::Passthrough {
                enrichment: Some(format!(
                    "[verdict_handler] VERDICT: block[{reason}] on {} but no active in_progress \
                     task found. Passing through to LLM.\n\n",
                    pr_url
                )),
            };
        }
    };

    let task_id = task.id.clone();

    // Mark task blocked
    if let Err(e) = db.update_task_status(&task_id, "blocked").await {
        warn!(error = %e, task_id = %task_id, "Failed to mark task blocked for block[{reason}]");
    }

    // Update metadata
    if let Err(e) = update_escalation_metadata(
        db,
        &task_id,
        &task.metadata,
        reason,
        &event.review_url,
        &truncate_body(&event.body),
    )
    .await
    {
        warn!(error = %e, task_id = %task_id, "Failed to update escalation metadata for block[{reason}]");
    }

    // Log audit event
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "verdict_escalated",
            &format!("task:{task_id}"),
            Some("in_progress"),
            Some("blocked"),
            Some(&format!(
                "verdict=block[{reason}] action=escalated pr_url={pr_url}"
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log verdict_escalated audit event for block[{reason}]");
    }

    // Notify operator
    if let Some(sender) = message_sender {
        send_notification(
            sender,
            &format!(
                "PR #{} on {} — VERDICT: block[{reason}] from @{}. \
                 Task {} marked blocked. Operator review required.",
                event.pr_number, event.repo, event.reviewer, task_id
            ),
        )
        .await;
    }

    info!(
        pr_number = event.pr_number,
        repo = %event.repo,
        task_id = %task_id,
        reason = %reason,
        "Structural verdict handler: block[{reason}] — escalation initiated"
    );

    VerdictAction::Handled {
        pre_digest: format_escalate_pre_digest(event, reason, &task_id),
    }
}

// ---------------------------------------------------------------------------
// Hold[review] handler (#889)
// ---------------------------------------------------------------------------

/// Handle VERDICT: hold[review] — notify operator, leave task in_progress.
/// Enriches the pre-digest with diff fingerprint data (#1563).
async fn handle_hold_review(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();

    let task = match find_task_for_verdict(db, &pr_url, event).await {
        Some(t) => t,
        None => {
            return VerdictAction::Passthrough {
                enrichment: Some(format!(
                    "[verdict_handler] VERDICT: hold[review] on {} but no active in_progress task \
                     found. Passing through to LLM.\n\n",
                    pr_url
                )),
            };
        }
    };

    let task_id = task.id.clone();

    // Fetch diff fingerprint for enrichment (fail-open — omit on error)
    let fingerprint_enrichment = if let Some(token) = github_token {
        match fetch_pr_head_sha(event.pr_number, &event.repo, token).await {
            Ok(sha) => {
                let history = read_diff_fingerprints(&task.metadata);
                let identical_count = history.iter().filter(|fp| fp.sha == sha).count() as u32;

                // Append to metadata
                if let Err(e) =
                    append_diff_fingerprint(db, &task_id, &task.metadata, &sha, "hold[review]")
                        .await
                {
                    warn!(error = %e, task_id = %task_id, "Failed to append hold[review] diff fingerprint");
                }

                Some((sha, identical_count))
            }
            Err(e) => {
                warn!(
                    error = %e,
                    pr_number = event.pr_number,
                    "Failed to fetch PR head SHA for hold[review] enrichment — omitting fingerprint data"
                );
                None
            }
        }
    } else {
        None
    };

    // Update metadata
    if let Err(e) = update_hold_metadata(
        db,
        &task_id,
        &task.metadata,
        &event.review_url,
        &truncate_body(&event.body),
    )
    .await
    {
        warn!(error = %e, task_id = %task_id, "Failed to update hold[review] metadata");
    }

    // Log audit event
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "verdict_held",
            &format!("task:{task_id}"),
            Some("in_progress"),
            None,
            Some(&format!("verdict=hold[review] pr_url={pr_url}")),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log verdict_held audit event");
    }

    // Notify operator
    if let Some(sender) = message_sender {
        send_notification(
            sender,
            &format!(
                "PR #{} on {} — VERDICT: hold[review] from @{}. \
                 Awaiting operator decision. Task: {}",
                event.pr_number, event.repo, event.reviewer, task_id
            ),
        )
        .await;
    }

    info!(
        pr_number = event.pr_number,
        repo = %event.repo,
        task_id = %task_id,
        "Structural verdict handler: hold[review] — operator notified"
    );

    VerdictAction::Handled {
        pre_digest: format_hold_review_pre_digest(event, &task_id, fingerprint_enrichment.as_ref()),
    }
}

// ---------------------------------------------------------------------------
// Missing verdict handler (#889)
// ---------------------------------------------------------------------------

/// Handle missing/unparseable VERDICT — safe-default hold[review] semantics.
async fn handle_missing_verdict(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    truncated: bool,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let pr_url = event.pr_url();
    let body_excerpt = truncate_body(&event.body);

    // Log structured verdict_classification_failed event
    warn!(
        pr_number = event.pr_number,
        repo = %event.repo,
        reviewer = %event.reviewer,
        review_url = %event.review_url,
        body_truncated = truncated,
        body_excerpt = %truncate_body_for_log(&body_excerpt),
        "verdict_classification_failed: no parseable VERDICT: line in review body"
    );

    // Look up task — if found, update metadata; if not, still handle structurally
    if let Some(task) = find_task_for_verdict(db, &pr_url, event).await {
        let task_id = task.id.clone();

        if let Err(e) = update_hold_metadata(
            db,
            &task_id,
            &task.metadata,
            &event.review_url,
            &body_excerpt,
        )
        .await
        {
            warn!(error = %e, task_id = %task_id, "Failed to update hold metadata for missing verdict");
        }

        // Log audit event
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "verdict_classification_failed",
                &format!("task:{task_id}"),
                Some(&task.status),
                None,
                Some(&format!(
                    "pr_url={pr_url} review_url={} body_truncated={truncated}",
                    event.review_url
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log verdict_classification_failed audit event");
        }

        // Notify operator
        if let Some(sender) = message_sender {
            send_notification(
                sender,
                &format!(
                    "PR #{} on {} — no parseable VERDICT line in review from @{}. \
                     Operator attention needed. Task: {}",
                    event.pr_number, event.repo, event.reviewer, task_id
                ),
            )
            .await;
        }
    } else {
        // No task found — still log the classification failure
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "verdict_classification_failed",
                &format!("pr:{}", pr_url),
                None,
                None,
                Some(&format!(
                    "pr_url={pr_url} review_url={} body_truncated={truncated} no_task=true",
                    event.review_url
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log verdict_classification_failed audit event (no task)");
        }

        if let Some(sender) = message_sender {
            send_notification(
                sender,
                &format!(
                    "PR #{} on {} — no parseable VERDICT line in review from @{}. \
                     No active task found. Operator attention needed.",
                    event.pr_number, event.repo, event.reviewer
                ),
            )
            .await;
        }
    }

    VerdictAction::Handled {
        pre_digest: format_verdict_classification_failed_pre_digest(event),
    }
}

// ---------------------------------------------------------------------------
// PR-keyed circuit breaker (mika#1563)
// ---------------------------------------------------------------------------

/// PR-keyed circuit breaker — fires regardless of task lookup outcome.
///
/// Returns `Some(VerdictAction::Handled)` when the breaker trips; `None` when
/// the verdict should continue through normal dispatch. Logs a
/// `verdict_observed` audit event on every invocation so the count survives
/// across syncs.
///
/// The breaker uses two timestamp anchors:
/// - `now` for the new audit row's `created_at` (handled by SQLite default)
/// - `since = now - PR_CIRCUIT_BREAKER_WINDOW_SECS` for the count query's lower bound
///
/// Counting strictly-prior events (before this invocation's audit insert)
/// keeps the threshold check inclusive: the 3rd observation in the window
/// trips on its own audit row's count, not on a future event.
async fn pr_circuit_breaker_check(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> Option<VerdictAction> {
    let pr_url = event.pr_url();
    let since =
        crate::timestamp::now_minus(chrono::Duration::seconds(PR_CIRCUIT_BREAKER_WINDOW_SECS));

    // Count prior observations BEFORE writing this one. The current invocation
    // is the (count+1)th observation; trip when count >= threshold means
    // strictly more than threshold observations have accumulated counting
    // this one.
    let prior_count = match db
        .count_recent_audit_events_for_target(VERDICT_OBSERVED_TOOL, &pr_url, &since)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            // Fail-open: DB errors here must NOT block the verdict pipeline.
            // The per-task gate is the primary defense; this is a structural
            // backstop for the lookup-failure case. A breaker that becomes
            // its own outage surface is worse than no breaker.
            warn!(
                error = %e,
                pr_url = %pr_url,
                "pr_circuit_breaker: count query failed — failing open"
            );
            0
        }
    };

    // Record this observation. Fire-and-forget per the C2.3 log-and-skip
    // convention — failure to record a single observation must not break
    // the verdict pipeline.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            VERDICT_OBSERVED_TOOL,
            &pr_url,
            None,
            None,
            Some(&format!(
                "pr_number={} repo={} reviewer={}",
                event.pr_number, event.repo, event.reviewer,
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(
            error = %e,
            pr_url = %pr_url,
            "pr_circuit_breaker: failed to log verdict_observed audit event"
        );
    }

    // Trip when the count of prior observations plus this one reaches the
    // threshold. The +1 accounts for the just-recorded observation.
    let total = prior_count + 1;
    if total < PR_CIRCUIT_BREAKER_THRESHOLD {
        return None;
    }

    info!(
        event = "pr_circuit_breaker_tripped",
        pr_number = event.pr_number,
        repo = %event.repo,
        pr_url = %pr_url,
        observation_count = total,
        threshold = PR_CIRCUIT_BREAKER_THRESHOLD,
        window_secs = PR_CIRCUIT_BREAKER_WINDOW_SECS,
        "PR-keyed circuit breaker tripped — refusing dispatch + notifying operator"
    );

    // Loud audit event so operator surfaces (dashboard, mika-platform monitor)
    // can detect the trip without grepping logs.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "verdict_circuit_breaker_tripped",
            &pr_url,
            None,
            None,
            Some(&format!(
                "observation_count={total} threshold={PR_CIRCUIT_BREAKER_THRESHOLD} \
                 window_secs={PR_CIRCUIT_BREAKER_WINDOW_SECS}"
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "pr_circuit_breaker: failed to log trip audit event");
    }

    if let Some(sender) = message_sender {
        send_notification(
            sender,
            &format!(
                "PR #{} on {} — circuit breaker tripped after {} blocking-verdict observations \
                 in {} min. Latest from @{}. Autonomous dispatch halted — operator triage required.",
                event.pr_number,
                event.repo,
                total,
                PR_CIRCUIT_BREAKER_WINDOW_SECS / 60,
                event.reviewer,
            ),
        )
        .await;
    }

    Some(VerdictAction::Handled {
        pre_digest: format_circuit_breaker_pre_digest(event, total),
    })
}

/// Pre-digest emitted when the circuit breaker trips. Replaces the original
/// verdict text so the LLM sees a halt signal instead of a fresh block[*]
/// instruction. Avoids completion-claim trigger words.
fn format_circuit_breaker_pre_digest(event: &PrReviewEvent, count: i64) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         \n\
         CIRCUIT BREAKER TRIPPED: {} blocking-verdict observations on this PR within \
         {} minutes. Per mika#1563, autonomous dispatch on this PR is now halted to \
         prevent token-burn loops.\n\
         \n\
         Operator has been notified. Do NOT dispatch run_claude_pilot for this PR. \
         Acknowledge the halt and surface the PR URL ({}) for operator triage.\n\
         </verdict_handler>",
        event.state,
        event.repo,
        event.pr_number,
        event.reviewer,
        count,
        PR_CIRCUIT_BREAKER_WINDOW_SECS / 60,
        event.pr_url(),
    )
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Look up an active in_progress task by PR URL. Returns None if not found
/// or task is not in_progress.
async fn find_task_for_verdict(
    db: &AsyncDatabase,
    pr_url: &str,
    event: &PrReviewEvent,
) -> Option<crate::db::Task> {
    match db.find_active_task_by_pr_url(pr_url).await {
        Ok(Some(t)) if t.status == "in_progress" => Some(t),
        Ok(Some(t)) => {
            info!(
                task_id = %t.id,
                status = %t.status,
                pr_url = %pr_url,
                "Verdict handler: task found but not in_progress (status: {})",
                t.status
            );
            None
        }
        Ok(None) => {
            info!(
                pr_number = event.pr_number,
                repo = %event.repo,
                pr_url = %pr_url,
                "Verdict handler: no active task found for PR"
            );
            None
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_url = %pr_url,
                "Failed to look up task by PR URL"
            );
            None
        }
    }
}

/// Read a verdict retry counter from task metadata JSON.
fn read_verdict_retry_count(metadata: &Option<String>, key: &str) -> u32 {
    metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get(key)?.get("count")?.as_u64())
        .unwrap_or(0) as u32
}

// ---------------------------------------------------------------------------
// Identical-diff circuit breaker helpers (mika#1563)
// ---------------------------------------------------------------------------

/// Fetch the current head commit SHA for a PR via `gh pr view`.
async fn fetch_pr_head_sha(pr_number: u64, repo: &str, token: &str) -> Result<String, String> {
    let pr_str = pr_number.to_string();
    let args = vec![
        "pr",
        "view",
        &pr_str,
        "--repo",
        repo,
        "--json",
        "headRefOid",
        "--jq",
        ".headRefOid",
    ];

    let future = run_gh_subprocess(&args, token);
    match tokio::time::timeout(std::time::Duration::from_secs(15), future).await {
        Ok(Ok(output)) => {
            let sha = output.trim().to_string();
            if sha.is_empty() {
                Err("gh pr view returned empty headRefOid".to_string())
            } else {
                Ok(sha)
            }
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("gh pr view timed out after 15s".to_string()),
    }
}

/// Read the diff fingerprint history from task metadata JSON.
fn read_diff_fingerprints(metadata: &Option<String>) -> Vec<DiffFingerprint> {
    metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("verdict_diff_fingerprints").cloned())
        .and_then(|v| serde_json::from_value::<Vec<DiffFingerprint>>(v).ok())
        .unwrap_or_default()
}

/// Append a diff fingerprint entry to task metadata.
async fn append_diff_fingerprint(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    sha: &str,
    verdict_class: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let new_entry = json!({
        "sha": sha,
        "verdict": verdict_class,
        "at": crate::timestamp::now(),
    });

    // Append to existing array or create new one
    let arr = base
        .as_object_mut()
        .unwrap()
        .entry("verdict_diff_fingerprints")
        .or_insert_with(|| json!([]));

    if let Some(arr) = arr.as_array_mut() {
        arr.push(new_entry);
    }

    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

/// Check the identical-diff circuit breaker. Returns `Some(VerdictAction)` when
/// the circuit breaker fires (same SHA rejected ≥ IDENTICAL_DIFF_THRESHOLD times),
/// or `None` to fall through to the generic retry counter.
#[allow(clippy::too_many_arguments)]
async fn check_identical_diff_circuit_breaker(
    event: &PrReviewEvent,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    task_id: &str,
    task_metadata: &Option<String>,
    pr_url: &str,
    verdict_class: &str,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> Option<VerdictAction> {
    // Require GitHub token — fail-open on None
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                pr_number = event.pr_number,
                repo = %event.repo,
                "identical-diff circuit breaker skipped — no GitHub token available"
            );
            return None;
        }
    };

    // Fetch current PR head SHA
    let sha = match fetch_pr_head_sha(event.pr_number, &event.repo, token).await {
        Ok(s) => s,
        Err(e) => {
            warn!(
                error = %e,
                pr_number = event.pr_number,
                repo = %event.repo,
                "identical-diff circuit breaker skipped — failed to fetch head SHA"
            );
            return None;
        }
    };

    // Read existing fingerprint history
    let history = read_diff_fingerprints(task_metadata);
    let identical_count = history.iter().filter(|fp| fp.sha == sha).count() as u32;

    if identical_count >= IDENTICAL_DIFF_THRESHOLD {
        // Circuit breaker fires
        info!(
            event = "identical_diff_circuit_breaker",
            pr_number = event.pr_number,
            repo = %event.repo,
            task_id = %task_id,
            head_sha = %sha,
            identical_count = identical_count,
            verdict_class = %verdict_class,
            trace_id = %trace_id,
            "Identical-diff circuit breaker fired — same SHA rejected {}x",
            identical_count
        );

        // Mark task blocked
        if let Err(e) = db.update_task_status(task_id, "blocked").await {
            warn!(error = %e, task_id = %task_id, "Failed to mark task blocked after identical-diff circuit breaker");
        }

        // Write circuit breaker metadata
        let cb_metadata = json!({
            "identical_diff_circuit_breaker": {
                "head_sha": sha,
                "identical_count": identical_count,
                "verdict_class": verdict_class,
                "fired_at": crate::timestamp::now(),
            }
        });
        let mut base = task_metadata
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .unwrap_or_else(|| json!({}));
        merge_metadata(&mut base, &cb_metadata);
        if let Ok(merged_str) = serde_json::to_string(&base)
            && let Err(e) = db.update_task_metadata(task_id, &merged_str).await
        {
            warn!(error = %e, task_id = %task_id, "Failed to update circuit breaker metadata");
        }

        // Audit event
        let matching_fingerprints: Vec<&DiffFingerprint> =
            history.iter().filter(|fp| fp.sha == sha).collect();
        let verdict_history_json =
            serde_json::to_string(&matching_fingerprints).unwrap_or_default();
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "verdict_handler",
                "identical_diff_circuit_breaker",
                Some("in_progress"),
                Some("blocked"),
                Some(&format!(
                    "pr_url={pr_url} head_sha={sha} identical_count={identical_count} \
                     verdict_history={verdict_history_json}"
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log identical_diff_circuit_breaker audit event");
        }

        // Notify operator
        if let Some(sender) = message_sender {
            send_notification(
                sender,
                &format!(
                    "PR #{} on {} — identical-diff circuit breaker fired ({verdict_class}). \
                     Same diff (SHA: {}) rejected {identical_count}x. \
                     Task {task_id} marked blocked. Operator review required.",
                    event.pr_number,
                    event.repo,
                    &sha[..8.min(sha.len())]
                ),
            )
            .await;
        }

        return Some(VerdictAction::Handled {
            pre_digest: format_identical_diff_pre_digest(
                event,
                task_id,
                &sha,
                identical_count,
                verdict_class,
            ),
        });
    }

    // Below threshold — append fingerprint and fall through
    if let Err(e) = append_diff_fingerprint(db, task_id, task_metadata, &sha, verdict_class).await {
        warn!(error = %e, task_id = %task_id, "Failed to append diff fingerprint");
    }

    None
}

/// Truncate body to BODY_EXCERPT_MAX chars, appending [truncated] if needed.
fn truncate_body(body: &str) -> String {
    if body.len() > BODY_EXCERPT_MAX {
        // Find a valid char boundary near BODY_EXCERPT_MAX
        let boundary = body
            .char_indices()
            .take_while(|&(i, _)| i <= BODY_EXCERPT_MAX)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(BODY_EXCERPT_MAX);
        format!("{}[truncated]", &body[..boundary])
    } else {
        body.to_string()
    }
}

/// UTF-8 safe truncation for log fields (max 200 chars).
fn truncate_body_for_log(body: &str) -> &str {
    if body.len() <= 200 {
        return body;
    }
    // Find a valid char boundary at or before byte offset 200
    let boundary = body
        .char_indices()
        .take_while(|&(i, _)| i < 200)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    &body[..boundary]
}

/// Send a notification via the message sender, absorbing errors.
async fn send_notification(sender: &Arc<dyn MessageSender>, message: &str) {
    match sender.send(message).await {
        Ok(crate::messaging::SendOutcome::Delivered) => {}
        Ok(crate::messaging::SendOutcome::Failed { reason }) => {
            warn!(reason = %reason, "Verdict handler notification delivery failed");
        }
        Ok(crate::messaging::SendOutcome::NoChannel) => {
            warn!("Verdict handler notification skipped — no reply channel (chat_id=0)");
        }
        Err(e) => {
            warn!(error = %e, "Failed to send verdict handler notification");
        }
    }
}

// ---------------------------------------------------------------------------
// AC extraction (#889 Change 5)
// ---------------------------------------------------------------------------

/// Regex for structured AC lines from qa-review: `- [❌] unsatisfied: <text>`
static AC_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match lines like: - [❌] unsatisfied: <AC text>: <details>
    // Also handle variants with extra whitespace
    Regex::new(r"(?m)^\s*-\s*\[❌\]\s*unsatisfied:\s*(.+)$").expect("ac line regex")
});

/// Result of AC extraction.
enum AcExtraction {
    /// Successfully parsed structured AC lines.
    Structured(Vec<String>),
    /// Fallback: parser yielded zero matches; using body excerpt.
    Fallback(String),
}

impl AcExtraction {
    fn summary(&self) -> String {
        match self {
            AcExtraction::Structured(acs) => {
                let items: Vec<String> = acs
                    .iter()
                    .enumerate()
                    .map(|(i, ac)| format!("  {}. {}", i + 1, ac))
                    .collect();
                format!("{} unsatisfied AC(s):\n{}", acs.len(), items.join("\n"))
            }
            AcExtraction::Fallback(excerpt) => {
                format!("[ac-extraction-fallback: true]\n{excerpt}")
            }
        }
    }

    fn is_fallback(&self) -> bool {
        matches!(self, AcExtraction::Fallback(_))
    }
}

/// Extract unsatisfied ACs from verdict body, with fallback to body excerpt.
fn extract_ac_list_or_fallback(body: &str) -> AcExtraction {
    let acs: Vec<String> = AC_LINE_RE
        .captures_iter(body)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .collect();

    if acs.is_empty() {
        AcExtraction::Fallback(truncate_body(body))
    } else {
        AcExtraction::Structured(acs)
    }
}

// ---------------------------------------------------------------------------
// Metadata update helpers
// ---------------------------------------------------------------------------

/// Update task metadata with verdict merge state (existing from #524).
async fn update_verdict_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    action: &str,
    pr_number: u64,
    pr_url: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "verdict_merge": {
            "state": action,
            "pr_number": pr_number,
            "pr_url": pr_url,
            "handled_at": crate::timestamp::now(),
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

/// Update task metadata for block[ac] or block[ci] verdicts.
#[allow(clippy::too_many_arguments)]
async fn update_verdict_block_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    key: &str,
    count: u32,
    pr_url: &str,
    review_url: &str,
    body_excerpt: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        key: {
            "count": count,
            "received_at": crate::timestamp::now(),
            "review_url": review_url,
            "pr_url": pr_url,
            "last_verdict_body_excerpt": body_excerpt,
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

/// Update task metadata for escalation (block[security]/block[pipeline]).
async fn update_escalation_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    reason: &str,
    review_url: &str,
    body_excerpt: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "verdict_escalated": {
            "reason": reason,
            "received_at": crate::timestamp::now(),
            "review_url": review_url,
            "body_excerpt": body_excerpt,
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

/// Update task metadata for hold[review] or missing verdict.
async fn update_hold_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    review_url: &str,
    body_excerpt: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "verdict_hold_review": {
            "received_at": crate::timestamp::now(),
            "review_url": review_url,
            "body_excerpt": body_excerpt,
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------

/// Format the pre-digest message for a successful merge action.
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses "initiated" / "enabled" phrasing.
fn format_success_pre_digest(event: &PrReviewEvent, action: &str, task_id: &str) -> String {
    let action_text = match action {
        "auto_merge_enabled" => format!(
            "Auto-merge has been enabled for PR #{} on {}. \
             GitHub will finalize the squash-merge once all CI checks pass.",
            event.pr_number, event.repo
        ),
        "merge_initiated" => format!(
            "Squash-merge has been initiated for PR #{} on {} (branch deletion requested).",
            event.pr_number, event.repo
        ),
        _ => format!(
            "Structural merge action '{}' taken on PR #{} on {}.",
            action, event.pr_number, event.repo
        ),
    };

    format!(
        "<verdict_handler>\n\
         [GitHub] PR review (approved) on {}#{} by @{}\n\
         VERDICT: pass — structural handler acted.\n\n\
         {action_text}\n\n\
         Task: {task_id}\n\
         Review: {}\n\n\
         Do NOT call pr_merge_with_gate — the merge action is already in progress.\n\
         Update the task status to reflect the outcome, then notify the user.\n\
         </verdict_handler>",
        event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format the pre-digest for a PR that was already merged.
fn format_already_merged_pre_digest(event: &PrReviewEvent, task_id: &str) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review (approved) on {}#{} by @{}\n\
         VERDICT: pass — PR was already finalized before the handler ran.\n\n\
         Task: {task_id}\n\
         Review: {}\n\n\
         Do NOT call pr_merge_with_gate — no action needed.\n\
         Update the task status if not already done, then notify the user.\n\
         </verdict_handler>",
        event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format the pre-digest for a merge error.
fn format_error_pre_digest(event: &PrReviewEvent, error: &str) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review (approved) on {}#{} by @{}\n\
         VERDICT: pass — structural handler attempted merge but encountered an error.\n\n\
         Error: {error}\n\n\
         The structural merge handler could not finalize the merge. \
         Investigate the error and decide the next action.\n\
         </verdict_handler>",
        event.repo, event.pr_number, event.reviewer
    )
}

/// Format pre-digest for block[ac] dispatch.
fn format_block_ac_pre_digest(
    event: &PrReviewEvent,
    ac_summary: &str,
    task_id: &str,
    retry_count: u32,
) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: block[ac] — structural handler preparing AC-fix dispatch.\n\n\
         {ac_summary}\n\n\
         Task: {task_id}\n\
         Retry: {retry_count}/{BLOCK_AC_MAX_RETRIES}\n\
         Review: {}\n\n\
         Action required: dispatch run_claude_pilot with skill: \"dev-pilot\" to fix the \
         unsatisfied ACs listed above. Include the verdict body and AC list in the \
         iteration_context for the child session.\n\
         Do NOT re-increment the retry counter — the structural handler already updated it.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for block[ac] retry limit reached.
fn format_block_ac_limit_pre_digest(event: &PrReviewEvent, task_id: &str) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: block[ac] — AC-fix retry limit ({BLOCK_AC_MAX_RETRIES}) reached.\n\n\
         Task: {task_id} (now blocked)\n\
         Review: {}\n\n\
         Do NOT dispatch run_claude_pilot — the retry budget is exhausted.\n\
         Notify the user about the persistent AC failures and ask for manual intervention.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for block[ci] dispatch.
fn format_block_ci_pre_digest(
    event: &PrReviewEvent,
    ci_context: &str,
    task_id: &str,
    retry_count: u32,
) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: block[ci] — structural handler preparing CI-fix dispatch.\n\n\
         CI failure context:\n{ci_context}\n\n\
         Task: {task_id}\n\
         Retry: {retry_count}/{BLOCK_CI_MAX_RETRIES}\n\
         Review: {}\n\n\
         Action required: dispatch run_claude_pilot with skill: \"dev-pilot\" to fix the \
         CI failures. Include the verdict body in the iteration_context for the child session.\n\
         Do NOT re-increment the retry counter — the structural handler already updated it.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for block[ci] retry limit reached.
fn format_block_ci_limit_pre_digest(event: &PrReviewEvent, task_id: &str) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: block[ci] — CI-fix retry limit ({BLOCK_CI_MAX_RETRIES}) reached.\n\n\
         Task: {task_id} (now blocked)\n\
         Review: {}\n\n\
         Do NOT dispatch run_claude_pilot — the retry budget is exhausted.\n\
         Notify the user about the persistent CI failures and ask for manual intervention.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for block[security] / block[pipeline] escalation.
fn format_escalate_pre_digest(event: &PrReviewEvent, reason: &str, task_id: &str) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: block[{reason}] — escalation initiated.\n\n\
         Task: {task_id} (now blocked)\n\
         Review: {}\n\n\
         Do NOT dispatch run_claude_pilot — block[{reason}] requires operator review.\n\
         Notify the user about the block[{reason}] verdict and request manual intervention.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for hold[review], with optional diff fingerprint enrichment (#1563).
fn format_hold_review_pre_digest(
    event: &PrReviewEvent,
    task_id: &str,
    fingerprint: Option<&(String, u32)>,
) -> String {
    let fingerprint_section = match fingerprint {
        Some((sha, count)) if *count >= IDENTICAL_DIFF_THRESHOLD => {
            format!(
                "\nDiff fingerprint: {sha}\n\
                 Identical-diff rejection count: {count}/{IDENTICAL_DIFF_THRESHOLD}\n\
                 IDENTICAL DIFF CIRCUIT BREAKER: same diff rejected {count}x — \
                 DO NOT dispatch run_claude_pilot.\n"
            )
        }
        Some((sha, count)) => {
            format!(
                "\nDiff fingerprint: {sha}\n\
                 Identical-diff rejection count: {count}/{IDENTICAL_DIFF_THRESHOLD}\n"
            )
        }
        None => String::new(),
    };

    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: hold[review] — operator notified; task remains in_progress.\n\n\
         Task: {task_id}\n\
         Review: {}{fingerprint_section}\n\
         Do NOT dispatch run_claude_pilot or take autonomous action.\n\
         The operator will decide the next move. Acknowledge the hold verdict.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for identical-diff circuit breaker firing (#1563).
fn format_identical_diff_pre_digest(
    event: &PrReviewEvent,
    task_id: &str,
    head_sha: &str,
    identical_count: u32,
    verdict_class: &str,
) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         VERDICT: {verdict_class} — identical diff circuit breaker fired.\n\n\
         The same PR diff (head SHA: {head_sha}) has been rejected {identical_count} times \
         across QA review cycles. The fix attempts are not producing new code changes.\n\n\
         Task: {task_id} (now blocked)\n\
         Review: {}\n\n\
         Do NOT dispatch run_claude_pilot — the identical-diff circuit breaker has halted \
         re-dispatch. The fix loop is stuck producing the same code.\n\
         Notify the user about the convergence failure and ask for manual intervention \
         (amend the plan, rewrite the QA criteria, or close the PR).\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format pre-digest for missing/unparseable verdict.
fn format_verdict_classification_failed_pre_digest(event: &PrReviewEvent) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review ({}) on {}#{} by @{}\n\
         [verdict_classification_failed] No parseable VERDICT: line in review body.\n\n\
         Review: {}\n\n\
         Operator has been notified. Do NOT take autonomous action on this review.\n\
         Acknowledge the classification failure and wait for operator guidance.\n\
         </verdict_handler>",
        event.state, event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Verify pre-digest messages don't trigger the completion-claim guard.
    // The guard regex: (?i)\b(merged|deployed|completed?|shipped)\b
    static COMPLETION_CLAIM_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?i)\b(merged|deployed|completed?|shipped)\b")
                .expect("completion claim regex")
        });

    fn sample_event() -> PrReviewEvent {
        PrReviewEvent {
            state: "approved".to_string(),
            repo: "senara-solutions/mika".to_string(),
            pr_number: 522,
            title: "fix: something".to_string(),
            reviewer: "mika-qa".to_string(),
            review_url: "https://github.com/senara-solutions/mika/pull/522#pullrequestreview-12345"
                .to_string(),
            body: "VERDICT: pass\n\nLooks good.".to_string(),
        }
    }

    fn sample_event_commented() -> PrReviewEvent {
        PrReviewEvent {
            state: "commented".to_string(),
            repo: "senara-solutions/mika".to_string(),
            pr_number: 888,
            title: "feat: something".to_string(),
            reviewer: "mika-qa".to_string(),
            review_url:
                "https://github.com/senara-solutions/mika/pull/888#pullrequestreview-4196247084"
                    .to_string(),
            body: "PLAN-AC VERIFICATION:\nPlan: docs/plans/test.md\nACs evaluated: 3\n\
                   - [\u{2705}] satisfied: AC1\n\
                   - [\u{274c}] unsatisfied: Unit 1 eval test: expected eval harness test; actual: missing\n\
                   - [\u{274c}] unsatisfied: Unit 3 operator command: expected mika-platform command; actual: missing\n\n\
                   VERDICT: block[ac]\n\
                   REASON: Two plan ACs unsatisfied."
                .to_string(),
        }
    }

    // ---- Pre-digest formatting tests ----

    #[test]
    fn success_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, "merge_initiated", "task-123");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn success_pre_digest_auto_merge_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, "auto_merge_enabled", "task-123");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn already_merged_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_already_merged_pre_digest(&event, "task-123");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn error_pre_digest_contains_error_message() {
        let event = sample_event();
        let text = format_error_pre_digest(&event, "Merge conflicts");
        assert!(text.contains("Merge conflicts"));
        assert!(text.contains("verdict_handler"));
    }

    #[test]
    fn success_pre_digest_contains_do_not_call_instruction() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, "merge_initiated", "task-123");
        assert!(text.contains("Do NOT call pr_merge_with_gate"));
    }

    #[test]
    fn success_pre_digest_contains_task_id() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, "merge_initiated", "task-123");
        assert!(text.contains("task-123"));
    }

    // ---- Block[ac] pre-digest tests ----

    #[test]
    fn block_ac_pre_digest_avoids_completion_claim_words() {
        let event = sample_event_commented();
        let text = format_block_ac_pre_digest(
            &event,
            "2 unsatisfied AC(s):\n  1. Unit 1\n  2. Unit 3",
            "task-456",
            1,
        );
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "block[ac] pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn block_ac_pre_digest_contains_ac_summary() {
        let event = sample_event_commented();
        let text = format_block_ac_pre_digest(
            &event,
            "2 unsatisfied AC(s):\n  1. Unit 1\n  2. Unit 3",
            "task-456",
            1,
        );
        assert!(text.contains("2 unsatisfied AC(s)"));
        assert!(text.contains("Unit 1"));
        assert!(text.contains("Unit 3"));
    }

    #[test]
    fn block_ac_pre_digest_contains_retry_count() {
        let event = sample_event_commented();
        let text = format_block_ac_pre_digest(&event, "1 AC", "task-456", 2);
        assert!(text.contains("Retry: 2/3"));
    }

    #[test]
    fn block_ac_pre_digest_contains_dispatch_instruction() {
        let event = sample_event_commented();
        let text = format_block_ac_pre_digest(&event, "1 AC", "task-456", 1);
        assert!(text.contains("dispatch run_claude_pilot"));
        assert!(text.contains("Do NOT re-increment the retry counter"));
    }

    #[test]
    fn block_ac_limit_pre_digest_avoids_completion_claim_words() {
        let event = sample_event_commented();
        let text = format_block_ac_limit_pre_digest(&event, "task-456");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "block[ac] limit pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn block_ac_limit_pre_digest_contains_no_dispatch_instruction() {
        let event = sample_event_commented();
        let text = format_block_ac_limit_pre_digest(&event, "task-456");
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
        assert!(text.contains("retry budget is exhausted"));
    }

    // ---- Block[ci] pre-digest tests ----

    #[test]
    fn block_ci_pre_digest_avoids_completion_claim_words() {
        let mut event = sample_event_commented();
        event.body = "VERDICT: block[ci]\nREASON: CI checks failing.".to_string();
        let text = format_block_ci_pre_digest(&event, "CI checks failing", "task-789", 1);
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "block[ci] pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn block_ci_pre_digest_contains_dispatch_instruction() {
        let mut event = sample_event_commented();
        event.body = "VERDICT: block[ci]\nREASON: CI checks failing.".to_string();
        let text = format_block_ci_pre_digest(&event, "CI checks failing", "task-789", 1);
        assert!(text.contains("dispatch run_claude_pilot"));
        assert!(text.contains("CI failures"));
    }

    #[test]
    fn block_ci_limit_pre_digest_avoids_completion_claim_words() {
        let mut event = sample_event_commented();
        event.body = "VERDICT: block[ci]".to_string();
        let text = format_block_ci_limit_pre_digest(&event, "task-789");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "block[ci] limit pre-digest contains completion-claim trigger word: {text}"
        );
    }

    // ---- Escalate pre-digest tests ----

    #[test]
    fn escalate_pre_digest_avoids_completion_claim_words() {
        let mut event = sample_event_commented();
        event.body = "VERDICT: block[security]\nREASON: Hardcoded secret.".to_string();
        let text = format_escalate_pre_digest(&event, "security", "task-sec");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "escalate pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn escalate_pre_digest_contains_no_dispatch_instruction() {
        let event = sample_event_commented();
        let text = format_escalate_pre_digest(&event, "security", "task-sec");
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
        assert!(text.contains("block[security]"));
        assert!(text.contains("operator review"));
    }

    #[test]
    fn escalate_pre_digest_pipeline() {
        let event = sample_event_commented();
        let text = format_escalate_pre_digest(&event, "pipeline", "task-pipe");
        assert!(text.contains("block[pipeline]"));
        assert!(text.contains("operator review"));
    }

    // ---- Hold[review] pre-digest tests ----

    #[test]
    fn hold_review_pre_digest_avoids_completion_claim_words() {
        let mut event = sample_event_commented();
        event.body = "VERDICT: hold[review]\nREASON: Needs design input.".to_string();
        let text = format_hold_review_pre_digest(&event, "task-hold", None);
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "hold[review] pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn hold_review_pre_digest_contains_no_action_instruction() {
        let event = sample_event_commented();
        let text = format_hold_review_pre_digest(&event, "task-hold", None);
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
        assert!(text.contains("remains in_progress"));
    }

    // ---- Missing verdict pre-digest tests ----

    #[test]
    fn verdict_classification_failed_pre_digest_avoids_completion_claim_words() {
        let mut event = sample_event_commented();
        event.body = "Just some comments, no verdict line.".to_string();
        let text = format_verdict_classification_failed_pre_digest(&event);
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "verdict_classification_failed pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn verdict_classification_failed_pre_digest_contains_classification_failed_tag() {
        let event = sample_event_commented();
        let text = format_verdict_classification_failed_pre_digest(&event);
        assert!(text.contains("verdict_classification_failed"));
        assert!(text.contains("No parseable VERDICT:"));
    }

    // ---- AC extraction tests ----

    #[test]
    fn extract_acs_from_structured_body() {
        let body = "PLAN-AC VERIFICATION:\n\
                    - [\u{2705}] satisfied: AC1\n\
                    - [\u{274c}] unsatisfied: Unit 1 eval test: expected eval harness test; actual: missing\n\
                    - [\u{274c}] unsatisfied: Unit 3 operator command: expected mika-platform command; actual: missing\n\n\
                    VERDICT: block[ac]";
        let result = extract_ac_list_or_fallback(body);
        match result {
            AcExtraction::Structured(acs) => {
                assert_eq!(acs.len(), 2);
                assert!(acs[0].contains("Unit 1 eval test"));
                assert!(acs[1].contains("Unit 3 operator command"));
            }
            AcExtraction::Fallback(_) => panic!("Expected structured extraction, got fallback"),
        }
    }

    #[test]
    fn extract_acs_fallback_on_no_matches() {
        let body = "Some review body without structured AC lines.\nVERDICT: block[ac]";
        let result = extract_ac_list_or_fallback(body);
        assert!(result.is_fallback());
        let summary = result.summary();
        assert!(summary.contains("ac-extraction-fallback: true"));
    }

    #[test]
    fn extract_acs_summary_format() {
        let body = "- [\u{274c}] unsatisfied: First AC: missing\n- [\u{274c}] unsatisfied: Second AC: wrong";
        let result = extract_ac_list_or_fallback(body);
        let summary = result.summary();
        assert!(summary.contains("2 unsatisfied AC(s)"));
        assert!(summary.contains("1. First AC: missing"));
        assert!(summary.contains("2. Second AC: wrong"));
    }

    // ---- Body truncation tests ----

    #[test]
    fn truncate_body_short() {
        let body = "Short body";
        assert_eq!(truncate_body(body), "Short body");
    }

    #[test]
    fn truncate_body_long() {
        let body = "x".repeat(3000);
        let result = truncate_body(&body);
        assert!(result.len() <= BODY_EXCERPT_MAX + "[truncated]".len() + 10);
        assert!(result.ends_with("[truncated]"));
    }

    #[test]
    fn truncate_body_exact_limit() {
        let body = "x".repeat(BODY_EXCERPT_MAX);
        assert_eq!(truncate_body(&body), body);
    }

    // ---- Retry counter tests ----

    #[test]
    fn read_verdict_retry_count_from_metadata() {
        let meta = Some(r#"{"verdict_block_ac": {"count": 2}}"#.to_string());
        assert_eq!(read_verdict_retry_count(&meta, "verdict_block_ac"), 2);
    }

    #[test]
    fn read_verdict_retry_count_missing_key() {
        let meta = Some(r#"{"other": "value"}"#.to_string());
        assert_eq!(read_verdict_retry_count(&meta, "verdict_block_ac"), 0);
    }

    #[test]
    fn read_verdict_retry_count_none_metadata() {
        assert_eq!(read_verdict_retry_count(&None, "verdict_block_ac"), 0);
    }

    #[test]
    fn read_verdict_retry_count_invalid_json() {
        let meta = Some("not json".to_string());
        assert_eq!(read_verdict_retry_count(&meta, "verdict_block_ac"), 0);
    }

    #[test]
    fn read_verdict_retry_count_ci() {
        let meta = Some(r#"{"verdict_block_ci": {"count": 1}}"#.to_string());
        assert_eq!(read_verdict_retry_count(&meta, "verdict_block_ci"), 1);
    }

    // ---- Verdict dispatch routing tests (unit-level, no async DB) ----

    #[test]
    fn verdict_pass_requires_approved_state() {
        // Verify that non-approved pass verdicts are caught by the state gate
        // (tested via the parse + dispatch logic, not full async handler)
        let body = "VERDICT: pass\nLooks good.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Pass);
        // The handler gates pass on event.state == "approved"
    }

    #[test]
    fn verdict_block_ac_dispatches_regardless_of_state() {
        // block[ac] dispatch is state-independent per #889
        let body = "VERDICT: block[ac]\nREASON: ACs unsatisfied.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("ac".to_string()));
        // The handler does NOT gate on state for block/hold verdicts
    }

    #[test]
    fn verdict_block_ci_parsed() {
        let body = "VERDICT: block[ci]\nREASON: CI failing.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("ci".to_string()));
    }

    #[test]
    fn verdict_block_security_parsed() {
        let body = "VERDICT: block[security]\nREASON: Hardcoded key.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("security".to_string()));
    }

    #[test]
    fn verdict_block_pipeline_parsed() {
        let body = "VERDICT: block[pipeline]\nREASON: Missing artifacts.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("pipeline".to_string()));
    }

    #[test]
    fn verdict_hold_review_parsed() {
        let body = "VERDICT: hold[review]\nREASON: Needs design input.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Hold("review".to_string()));
    }

    #[test]
    fn verdict_missing_detected() {
        let body = "Just a comment, no verdict.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Missing { truncated: false });
    }

    #[test]
    fn verdict_changes_requested_with_block_ac() {
        // R10: state=CHANGES_REQUESTED + block[ac] should dispatch (no double-fire)
        let body = "VERDICT: block[ac]\nREASON: ACs unsatisfied.";
        let verdict = parse_verdict(body);
        assert_eq!(verdict, Verdict::Block("ac".to_string()));
        // The handler dispatches block[ac] regardless of state — no special
        // CHANGES_REQUESTED path to double-fire from.
    }

    // ---- All pre-digest XML tag tests ----

    #[test]
    fn all_pre_digests_have_xml_tags() {
        let event = sample_event();
        let event_c = sample_event_commented();

        let pre_digests = vec![
            format_success_pre_digest(&event, "merge_initiated", "t1"),
            format_already_merged_pre_digest(&event, "t1"),
            format_error_pre_digest(&event, "error"),
            format_block_ac_pre_digest(&event_c, "1 AC", "t1", 1),
            format_block_ac_limit_pre_digest(&event_c, "t1"),
            format_block_ci_pre_digest(&event_c, "CI context", "t1", 1),
            format_block_ci_limit_pre_digest(&event_c, "t1"),
            format_escalate_pre_digest(&event_c, "security", "t1"),
            format_hold_review_pre_digest(&event_c, "t1", None),
            format_verdict_classification_failed_pre_digest(&event_c),
        ];

        for (i, text) in pre_digests.iter().enumerate() {
            assert!(
                text.contains("<verdict_handler>"),
                "Pre-digest #{i} missing opening XML tag: {text}"
            );
            assert!(
                text.contains("</verdict_handler>"),
                "Pre-digest #{i} missing closing XML tag: {text}"
            );
        }
    }

    // ---- All pre-digests avoid completion-claim words ----

    #[test]
    fn all_new_pre_digests_avoid_completion_claim_words() {
        let event_c = sample_event_commented();

        let pre_digests = vec![
            (
                "block_ac",
                format_block_ac_pre_digest(&event_c, "1 AC", "t1", 1),
            ),
            (
                "block_ac_limit",
                format_block_ac_limit_pre_digest(&event_c, "t1"),
            ),
            (
                "block_ci",
                format_block_ci_pre_digest(&event_c, "ctx", "t1", 1),
            ),
            (
                "block_ci_limit",
                format_block_ci_limit_pre_digest(&event_c, "t1"),
            ),
            (
                "escalate_security",
                format_escalate_pre_digest(&event_c, "security", "t1"),
            ),
            (
                "escalate_pipeline",
                format_escalate_pre_digest(&event_c, "pipeline", "t1"),
            ),
            (
                "hold_review",
                format_hold_review_pre_digest(&event_c, "t1", None),
            ),
            (
                "classification_failed",
                format_verdict_classification_failed_pre_digest(&event_c),
            ),
        ];

        for (name, text) in &pre_digests {
            assert!(
                !COMPLETION_CLAIM_RE.is_match(text),
                "{name} pre-digest contains completion-claim trigger word: {text}"
            );
        }
    }

    // ---- Diff fingerprint metadata helpers tests (mika#1563) ----

    #[test]
    fn read_diff_fingerprints_none_metadata() {
        let result = read_diff_fingerprints(&None);
        assert!(result.is_empty());
    }

    #[test]
    fn read_diff_fingerprints_missing_key() {
        let meta = Some(r#"{"other": "value"}"#.to_string());
        let result = read_diff_fingerprints(&meta);
        assert!(result.is_empty());
    }

    #[test]
    fn read_diff_fingerprints_populated_array() {
        let meta = Some(
            r#"{"verdict_diff_fingerprints": [
                {"sha": "abc123", "verdict": "block[ac]", "at": "2026-06-26T01:00:00Z"},
                {"sha": "def456", "verdict": "block[ci]", "at": "2026-06-26T01:30:00Z"}
            ]}"#
            .to_string(),
        );
        let result = read_diff_fingerprints(&meta);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].sha, "abc123");
        assert_eq!(result[0].verdict, "block[ac]");
        assert_eq!(result[1].sha, "def456");
        assert_eq!(result[1].verdict, "block[ci]");
    }

    #[test]
    fn read_diff_fingerprints_malformed_json() {
        let meta = Some("not json".to_string());
        let result = read_diff_fingerprints(&meta);
        assert!(result.is_empty());
    }

    #[test]
    fn read_diff_fingerprints_malformed_array_entries() {
        let meta = Some(r#"{"verdict_diff_fingerprints": [{"wrong_field": true}]}"#.to_string());
        // serde_json::from_value will fail on missing required fields, returning empty
        let result = read_diff_fingerprints(&meta);
        assert!(result.is_empty());
    }

    // ---- Identical-diff circuit breaker pre-digest tests ----

    #[test]
    fn identical_diff_pre_digest_avoids_completion_claim_words() {
        let event = sample_event_commented();
        let text =
            format_identical_diff_pre_digest(&event, "task-123", "abc123def", 3, "block[ac]");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "identical_diff pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn identical_diff_pre_digest_has_xml_tags() {
        let event = sample_event_commented();
        let text =
            format_identical_diff_pre_digest(&event, "task-123", "abc123def", 3, "block[ac]");
        assert!(text.contains("<verdict_handler>"));
        assert!(text.contains("</verdict_handler>"));
    }

    #[test]
    fn identical_diff_pre_digest_contains_key_fields() {
        let event = sample_event_commented();
        let text =
            format_identical_diff_pre_digest(&event, "task-123", "abc123def456", 4, "block[ci]");
        assert!(text.contains("identical diff"));
        assert!(text.contains("circuit breaker"));
        assert!(text.contains("abc123def456"));
        assert!(text.contains("4 times"));
        assert!(text.contains("task-123"));
        assert!(text.contains("block[ci]"));
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
    }

    #[test]
    fn identical_diff_pre_digest_no_dispatch_instruction() {
        let event = sample_event_commented();
        let text = format_identical_diff_pre_digest(&event, "task-123", "abc123", 3, "block[ac]");
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
        assert!(text.contains("circuit breaker has halted"));
    }

    // ---- Hold[review] with fingerprint enrichment tests ----

    #[test]
    fn hold_review_pre_digest_with_fingerprint_below_threshold() {
        let event = sample_event_commented();
        let fp = ("abc123def".to_string(), 1u32);
        let text = format_hold_review_pre_digest(&event, "task-hold", Some(&fp));
        assert!(text.contains("Diff fingerprint: abc123def"));
        assert!(text.contains("Identical-diff rejection count: 1/3"));
        assert!(!text.contains("CIRCUIT BREAKER"));
    }

    #[test]
    fn hold_review_pre_digest_with_fingerprint_at_threshold() {
        let event = sample_event_commented();
        let fp = ("abc123def".to_string(), 3u32);
        let text = format_hold_review_pre_digest(&event, "task-hold", Some(&fp));
        assert!(text.contains("IDENTICAL DIFF CIRCUIT BREAKER"));
        assert!(text.contains("DO NOT dispatch run_claude_pilot"));
    }

    #[test]
    fn hold_review_pre_digest_without_fingerprint() {
        let event = sample_event_commented();
        let text = format_hold_review_pre_digest(&event, "task-hold", None);
        assert!(!text.contains("Diff fingerprint"));
        assert!(!text.contains("CIRCUIT BREAKER"));
    }

    #[test]
    fn hold_review_pre_digest_with_fingerprint_avoids_completion_claim_words() {
        let event = sample_event_commented();
        let fp = ("abc123".to_string(), 3u32);
        let text = format_hold_review_pre_digest(&event, "task-hold", Some(&fp));
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "hold[review] pre-digest with fingerprint contains completion-claim trigger word: {text}"
        );
    }

    // ---- All pre-digests XML tags (including new ones) ----

    #[test]
    fn new_pre_digests_have_xml_tags() {
        let event_c = sample_event_commented();

        let pre_digests = [
            format_identical_diff_pre_digest(&event_c, "t1", "sha123", 3, "block[ac]"),
            format_hold_review_pre_digest(&event_c, "t1", Some(&("sha123".to_string(), 2))),
            format_hold_review_pre_digest(&event_c, "t1", Some(&("sha123".to_string(), 3))),
        ];

        for (i, text) in pre_digests.iter().enumerate() {
            assert!(
                text.contains("<verdict_handler>"),
                "New pre-digest #{i} missing opening XML tag: {text}"
            );
            assert!(
                text.contains("</verdict_handler>"),
                "New pre-digest #{i} missing closing XML tag: {text}"
            );
        }
    }
    // ---- PR-keyed circuit breaker tests (mika#1563) ----

    /// Construct an in-memory AsyncDatabase with an `agent_id` set, ready for
    /// audit_event writes. Mirrors the helper shape from `async_db::tests` but
    /// adds an explicit agent_id since `log_audit_event` requires it.
    async fn cb_test_db() -> AsyncDatabase {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = AsyncDatabase::new(db);
        // The agent_id is set by `AsyncDatabase::new` to "mika" by default;
        // we ensure the agent row exists for FK satisfaction on audit_events.
        async_db
            .with_db(|d| {
                d.conn.execute(
                    "INSERT OR IGNORE INTO agents (id, name) VALUES ('mika', 'mika')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        async_db
            .create_session("cb-test-session", "mika", "cli")
            .await
            .unwrap();
        async_db
    }

    fn cb_blocking_event(pr_number: u64) -> PrReviewEvent {
        PrReviewEvent {
            state: "commented".to_string(),
            repo: "senara-solutions/mika".to_string(),
            pr_number,
            title: "test plan".to_string(),
            reviewer: "mika-qa".to_string(),
            review_url: format!(
                "https://github.com/senara-solutions/mika/pull/{pr_number}#pullrequestreview-1"
            ),
            body: "VERDICT: block[ac]\nREASON: test".to_string(),
        }
    }

    #[tokio::test]
    async fn circuit_breaker_passes_below_threshold() {
        let db = cb_test_db().await;
        let event = cb_blocking_event(1100);

        // First and second observations should not trip — count is 1, then 2.
        for _ in 0..(PR_CIRCUIT_BREAKER_THRESHOLD - 1) {
            let action =
                pr_circuit_breaker_check(&event, &db, None, "cb-test-session", "trace-1").await;
            assert!(
                action.is_none(),
                "below threshold should return None, got Some"
            );
        }
    }

    #[tokio::test]
    async fn circuit_breaker_trips_at_threshold() {
        let db = cb_test_db().await;
        let event = cb_blocking_event(1101);

        // Run up to (threshold - 1) observations — all should pass.
        for _ in 0..(PR_CIRCUIT_BREAKER_THRESHOLD - 1) {
            let action =
                pr_circuit_breaker_check(&event, &db, None, "cb-test-session", "trace-2").await;
            assert!(action.is_none());
        }

        // The THRESHOLD-th observation must trip.
        let tripped =
            pr_circuit_breaker_check(&event, &db, None, "cb-test-session", "trace-2").await;
        assert!(
            matches!(tripped, Some(VerdictAction::Handled { .. })),
            "threshold observation must trip the breaker"
        );

        // The trip audit row was written.
        let trip_count = db
            .count_recent_audit_events_for_target(
                "verdict_circuit_breaker_tripped",
                &event.pr_url(),
                "1970-01-01T00:00:00Z",
            )
            .await
            .unwrap();
        assert_eq!(
            trip_count, 1,
            "expected one trip audit row, got {trip_count}"
        );
    }

    #[tokio::test]
    async fn circuit_breaker_isolates_pr_url() {
        // Observations on PR #A must not influence PR #B.
        let db = cb_test_db().await;
        let event_a = cb_blocking_event(1102);
        let event_b = cb_blocking_event(1103);

        // Saturate PR A to the threshold — last observation should trip.
        for _ in 0..PR_CIRCUIT_BREAKER_THRESHOLD {
            let _ =
                pr_circuit_breaker_check(&event_a, &db, None, "cb-test-session", "trace-3").await;
        }

        // PR B's first observation must NOT trip.
        let action_b =
            pr_circuit_breaker_check(&event_b, &db, None, "cb-test-session", "trace-3").await;
        assert!(
            action_b.is_none(),
            "PR B's first observation must not trip (saw {action_b:?})"
        );
    }

    #[tokio::test]
    async fn circuit_breaker_window_excludes_old_events() {
        // Observations older than the window should not count toward the trip.
        let db = cb_test_db().await;
        let event = cb_blocking_event(1104);
        let pr_url = event.pr_url();

        // Backdate threshold-worth of audit rows OUTSIDE the window.
        let outside_window = "2020-01-01T00:00:00Z";
        for _ in 0..(PR_CIRCUIT_BREAKER_THRESHOLD + 5) {
            db.with_db({
                let pr_url = pr_url.clone();
                let outside = outside_window.to_string();
                move |d| {
                    d.conn.execute(
                        "INSERT INTO audit_events
                         (agent_id, session_id, tool_name, target_key, created_at)
                         VALUES ('mika', 'cb-test-session', ?1, ?2, ?3)",
                        rusqlite::params![VERDICT_OBSERVED_TOOL, pr_url, outside],
                    )?;
                    Ok(())
                }
            })
            .await
            .unwrap();
        }

        // First in-window observation must not trip — only its own row is recent.
        let action =
            pr_circuit_breaker_check(&event, &db, None, "cb-test-session", "trace-4").await;
        assert!(
            action.is_none(),
            "old events outside the window should not count (saw {action:?})"
        );
    }
}
