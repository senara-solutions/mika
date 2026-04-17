//! Structural handler for `pull_request_review.submitted` webhook events.
//!
//! Intercepts PR review events **before** the LLM turn and acts on
//! `VERDICT: pass` deterministically (look up work item, initiate merge,
//! update metadata, audit, notify). This removes the merge decision from
//! LLM improvisation — it's a state-machine transition, not a judgement call.
//!
//! See issue #524 and the compound doc at
//! `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::tools::pr_merge_with_gate::{
    CheckClassification, classify_checks, run_gh_checks, run_gh_merge,
};
use crate::work_item_metadata::merge_metadata;

use super::verdict::{Verdict, parse_pr_review_event, parse_verdict};

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
/// Returns `VerdictAction::Handled` when the handler initiated a merge (or
/// encountered a merge error), with a pre-digest message for the LLM.
/// Returns `VerdictAction::Passthrough` for all other cases (non-review events,
/// non-approved reviews, block/hold verdicts, missing verdicts, missing work items).
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

    // 2. Only act on approved reviews
    if event.state != "approved" {
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 3. Parse the verdict
    let verdict = parse_verdict(&event.body);

    match verdict {
        Verdict::Block(_) | Verdict::Hold(_) => {
            // Let the LLM handle block/hold verdicts
            VerdictAction::Passthrough { enrichment: None }
        }
        Verdict::Missing { truncated } => {
            if truncated {
                warn!(
                    pr_number = event.pr_number,
                    repo = %event.repo,
                    "PR review body was truncated and no VERDICT: line found — possible data loss"
                );
            } else {
                warn!(
                    pr_number = event.pr_number,
                    repo = %event.repo,
                    reviewer = %event.reviewer,
                    "PR review approved but no VERDICT: line found (contract violation)"
                );
            }
            VerdictAction::Passthrough {
                enrichment: Some(
                    "[verdict_missing=true] No VERDICT: line found in approved review.\n\n"
                        .to_string(),
                ),
            }
        }
        Verdict::Pass => {
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
    }
}

/// Handle a VERDICT: pass — look up work item, initiate merge, update metadata.
async fn handle_pass_verdict(
    event: &super::verdict::PrReviewEvent,
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

    // Look up the work item by PR URL
    let work_item = match db.find_active_work_item_by_pr_url(&pr_url).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            info!(
                pr_number = event.pr_number,
                repo = %event.repo,
                pr_url = %pr_url,
                "VERDICT: pass but no active work item found for PR — passing through to LLM"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_url = %pr_url,
                "Failed to look up work item by PR URL — passing through to LLM"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // Only act when work item is in_progress
    if work_item.status != "in_progress" {
        info!(
            task_id = %work_item.id,
            status = %work_item.status,
            pr_url = %pr_url,
            "VERDICT: pass but work item not in_progress (status: {}) — skipping structural merge",
            work_item.status
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    let task_id = work_item.id.clone();

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

                    // Update work item metadata
                    if let Err(e) = update_verdict_metadata(
                        db,
                        &task_id,
                        &work_item.metadata,
                        action_desc,
                        event.pr_number,
                        &pr_url,
                    )
                    .await
                    {
                        warn!(error = %e, task_id = %task_id, "Failed to update work item metadata after merge");
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
                                "verdict=pass action={action_desc} pr_url={pr_url} work_item_id={task_id}"
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
                        match sender.send(&notification).await {
                            Ok(crate::messaging::SendOutcome::Delivered) => {}
                            Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                                warn!(reason = %reason, "Merge notification delivery failed");
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to send merge notification");
                            }
                        }
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

/// Update work item metadata with verdict merge state.
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
    db.update_work_item_metadata(task_id, &merged_str).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------

/// Format the pre-digest message for a successful merge action.
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses "initiated" / "enabled" phrasing.
fn format_success_pre_digest(
    event: &super::verdict::PrReviewEvent,
    action: &str,
    task_id: &str,
) -> String {
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
         Work item: {task_id}\n\
         Review: {}\n\n\
         Do NOT call pr_merge_with_gate — the merge action is already in progress.\n\
         Update the work item status to reflect the outcome, then notify the user.\n\
         </verdict_handler>",
        event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format the pre-digest for a PR that was already merged.
fn format_already_merged_pre_digest(
    event: &super::verdict::PrReviewEvent,
    task_id: &str,
) -> String {
    format!(
        "<verdict_handler>\n\
         [GitHub] PR review (approved) on {}#{} by @{}\n\
         VERDICT: pass — PR was already finalized before the handler ran.\n\n\
         Work item: {task_id}\n\
         Review: {}\n\n\
         Do NOT call pr_merge_with_gate — no action needed.\n\
         Update the work item status if not already done, then notify the user.\n\
         </verdict_handler>",
        event.repo, event.pr_number, event.reviewer, event.review_url
    )
}

/// Format the pre-digest for a merge error.
fn format_error_pre_digest(event: &super::verdict::PrReviewEvent, error: &str) -> String {
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

    fn sample_event() -> super::super::verdict::PrReviewEvent {
        super::super::verdict::PrReviewEvent {
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
    fn success_pre_digest_contains_work_item_id() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, "merge_initiated", "task-123");
        assert!(text.contains("task-123"));
    }
}
