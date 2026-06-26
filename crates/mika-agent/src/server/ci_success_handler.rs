//! Structural handler for `check_suite.completed(success)` webhook events.
//!
//! Intercepts CI success events **before** the LLM turn and re-evaluates
//! merge eligibility for PRs that have a pending `VERDICT: pass` but were
//! blocked on CI at the time of approval. This removes the merge recovery
//! from LLM improvisation — it's a state-machine transition, not a judgement call.
//!
//! Companion to `verdict_handler` (which handles `pull_request_review.submitted`).
//! See issue #571.
//!
//! ## Invariants
//!
//! - `check_suite.completed/success` is scoped to ONE workflow. A PR with
//!   multiple workflows can receive this event while another workflow is still
//!   running or already failed. We MUST aggregate across all required checks
//!   here — the webhook alone is not sufficient evidence that all CI is green.
//!   This is the actual gate, not a defensive re-check.
//!
//! - Structural handlers MUST return Passthrough for non-matching event types.
//!   Call order is irrelevant — handlers are disjoint on event_type.
//!
//! - Loop prevention: post-merge, GitHub fires `check_suite.completed/success`
//!   with `head_branch = main` (or whatever the base is). `gh pr list --head main
//!   --state open` returns empty → NoPr → no-op. Self-terminating by construction.
//!
//! - Stale verdict policy: if QA approved a different SHA than HEAD, we do NOT merge.
//!   A push after approval — even a mechanical fix — is unreviewed code.
//!   The cost is one extra QA cycle. The alternative is silently trusting
//!   that the push was safe, which is the judgment call we're trying to avoid here.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::task_state::merge_metadata;
use crate::tools::pr_merge_with_gate::{
    CheckClassification, classify_checks, is_behind_main, run_gh_checks, run_gh_merge,
    run_gh_pr_view, run_gh_subprocess,
};

use super::verdict::{Verdict, parse_verdict};
use super::verdict_handler::VerdictAction;

/// Parsed fields from a gateway-formatted check_suite success event.
pub(crate) struct CheckSuiteEvent {
    repo: String,
    branch: String,
}

/// Parse the gateway-formatted check_suite success event text.
///
/// Expected format: `[GitHub] Check suite success on {repo} (branch: {branch})`
pub(crate) fn parse_check_suite_success(text: &str) -> Option<CheckSuiteEvent> {
    let first_line = text.lines().next()?;

    // Must be a check_suite success event
    if !first_line.starts_with("[GitHub] Check suite success on ") {
        return None;
    }

    // Extract repo: between "on " and " (branch:"
    let after_on = first_line.strip_prefix("[GitHub] Check suite success on ")?;
    let branch_marker = " (branch: ";
    let marker_pos = after_on.find(branch_marker)?;
    let repo = &after_on[..marker_pos];

    // Extract branch: between "(branch: " and closing ")"
    let after_marker = &after_on[marker_pos + branch_marker.len()..];
    let branch = after_marker.strip_suffix(')')?;

    Some(CheckSuiteEvent {
        repo: repo.to_string(),
        branch: branch.to_string(),
    })
}

/// Attempt to handle a CI success event structurally before the LLM turn.
///
/// Returns `VerdictAction::Handled` when the handler initiated a merge (or
/// encountered a merge error), with a pre-digest message for the LLM.
/// Returns `VerdictAction::Passthrough` for all other cases (non-matching events,
/// no open PR, no QA verdict, stale verdict, checks not all green).
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ci_success(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Parse the check_suite success event from formatted text
    let event = match parse_check_suite_success(text) {
        Some(e) => e,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    info!(
        repo = %event.repo,
        branch = %event.branch,
        "CI success handler: evaluating merge eligibility"
    );

    // Require GitHub token for merge operations
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                repo = %event.repo,
                branch = %event.branch,
                "CI success but no GitHub token available — cannot evaluate merge"
            );
            return VerdictAction::Passthrough {
                enrichment: Some(
                    "[ci_success_handler] CI checks passed but no GitHub token configured. \
                     Cannot evaluate merge eligibility.\n\n"
                        .to_string(),
                ),
            };
        }
    };

    // 2. Find open PR for this branch
    let pr = match find_open_pr(&event.repo, &event.branch, token).await {
        Ok(Some(pr)) => pr,
        Ok(None) => {
            info!(
                repo = %event.repo,
                branch = %event.branch,
                "CI success but no open PR for branch — likely post-merge webhook"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                repo = %event.repo,
                branch = %event.branch,
                "Failed to look up PR for branch"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 3. Find QA pass verdict
    let verdict_review = match find_pass_verdict(pr.number, &event.repo, token).await {
        Ok(Some(review)) => review,
        Ok(None) => {
            info!(
                pr_number = pr.number,
                repo = %event.repo,
                "CI success but no VERDICT: pass review found on PR"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        Err(e) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                repo = %event.repo,
                "Failed to fetch PR reviews"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 4. Stale-SHA gate: review.commit_id must match pr.head.sha
    if verdict_review.commit_id != pr.head_sha {
        info!(
            pr_number = pr.number,
            repo = %event.repo,
            review_sha = %verdict_review.commit_id,
            head_sha = %pr.head_sha,
            reviewer = %verdict_review.reviewer,
            "CI success but VERDICT: pass was for a different SHA — stale verdict"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 5. CI aggregation gate (load-bearing — check_suite event is per-workflow)
    let checks_future = run_gh_checks(pr.number, &event.repo, token);
    let checks = match tokio::time::timeout(std::time::Duration::from_secs(60), checks_future).await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            warn!(
                error = %e,
                pr_number = pr.number,
                "Failed to fetch CI checks for CI success merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(&event, pr.number, &e),
            };
        }
        Err(_) => {
            warn!(
                pr_number = pr.number,
                "CI check fetch timed out after 60s for CI success merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(
                    &event,
                    pr.number,
                    "CI check fetch timed out after 60s",
                ),
            };
        }
    };

    let classification = classify_checks(&checks);

    if classification != CheckClassification::AllPassed {
        let detail = match classification {
            CheckClassification::HasFailures => {
                let failing: Vec<String> = checks
                    .iter()
                    .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
                    .map(|c| format!("  - {} ({})", c.name, c.state))
                    .collect();
                format!("failing checks:\n{}", failing.join("\n"))
            }
            CheckClassification::HasPending => {
                let pending: Vec<String> = checks
                    .iter()
                    .filter(|c| c.bucket == "pending")
                    .map(|c| format!("  - {} ({})", c.name, c.state))
                    .collect();
                format!("pending checks:\n{}", pending.join("\n"))
            }
            CheckClassification::AllPassed => unreachable!(),
        };

        info!(
            pr_number = pr.number,
            repo = %event.repo,
            "CI success event for one workflow but not all required checks pass yet: {detail}"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 5b. Behind-main assertion (#1577) — block merge if PR is behind main.
    // Fetch the PR's baseRefOid via gh pr view, then compare against main HEAD.
    // Fail-open: API errors log a warning and proceed.
    match run_gh_pr_view(pr.number, &event.repo, token).await {
        Ok(preflight) => {
            match is_behind_main(&preflight.base_ref_oid, &event.repo, token).await {
                Ok(Some(info)) => {
                    info!(
                        pr_number = pr.number,
                        pr_base_sha = %info.pr_base_sha,
                        current_main_sha = %info.current_main_sha,
                        "CI success handler: PR is behind main — skipping merge"
                    );
                    return VerdictAction::Passthrough {
                        enrichment: Some(format_behind_main_enrichment(
                            &info.pr_base_sha,
                            &info.current_main_sha,
                        )),
                    };
                }
                Ok(None) => {} // Up-to-date — proceed to merge
                Err(e) => {
                    warn!(
                        pr_number = pr.number,
                        error = %e,
                        "CI success handler: failed to check behind-main — proceeding (fail-open)"
                    );
                }
            }
        }
        Err(e) => {
            warn!(
                pr_number = pr.number,
                error = %e.message,
                "CI success handler: failed to fetch PR preflight for behind-main check — proceeding (fail-open)"
            );
        }
    }

    // 6. All conditions met — initiate merge
    // Look up task by PR URL for metadata update
    let pr_url = format!("https://github.com/{}/pull/{}", event.repo, pr.number);
    let task = match db.find_active_task_by_pr_url(&pr_url).await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, pr_url = %pr_url, "Failed to look up task by PR URL");
            None
        }
    };

    let merge_future = run_gh_merge(
        pr.number,
        &event.repo,
        "squash",
        true,  // delete_branch
        false, // not auto — we've verified all checks pass
        token,
    );
    let merge_result = tokio::time::timeout(std::time::Duration::from_secs(60), merge_future).await;

    let merge_result = match merge_result {
        Ok(inner) => inner,
        Err(_) => {
            warn!(pr_number = pr.number, "gh pr merge timed out after 60s");
            Err("gh pr merge timed out after 60s".to_string())
        }
    };

    match merge_result {
        Ok(_output) => {
            let task_id = task.as_ref().map(|t| t.id.as_str()).unwrap_or("none");

            // Update task metadata
            if let Some(ref t) = task
                && let Err(e) =
                    update_ci_merge_metadata(db, &t.id, &t.metadata, pr.number, &pr_url).await
            {
                warn!(error = %e, task_id = %t.id, "Failed to update task metadata after CI success merge");
            }

            // Log audit event
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "ci_success_merge",
                    &format!("task:{task_id}"),
                    Some("in_progress"),
                    Some("merge_initiated"),
                    Some(&format!(
                        "trigger=check_suite_success pr_url={pr_url} task_id={task_id}"
                    )),
                    Some(trace_id),
                )
                .await
            {
                warn!(error = %e, "Failed to log ci_success_merge audit event");
            }

            // Send notification
            if let Some(sender) = message_sender {
                let notification = format!(
                    "PR #{} on {} — CI checks all green + VERDICT: pass from @{}. \
                     Merge initiated (squash, delete branch).",
                    pr.number, event.repo, verdict_review.reviewer
                );
                match sender.send(&notification).await {
                    Ok(crate::messaging::SendOutcome::Delivered) => {}
                    Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                        warn!(reason = %reason, "CI success merge notification delivery failed");
                    }
                    Ok(crate::messaging::SendOutcome::NoChannel) => {
                        warn!(
                            "CI success merge notification skipped — no reply channel (chat_id=0)"
                        );
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to send CI success merge notification");
                    }
                }
            }

            info!(
                pr_number = pr.number,
                repo = %event.repo,
                task_id = task_id,
                reviewer = %verdict_review.reviewer,
                "CI success handler: merge initiated for PR #{} (QA pass from @{})",
                pr.number,
                verdict_review.reviewer
            );

            VerdictAction::Handled {
                pre_digest: format_success_pre_digest(
                    &event,
                    pr.number,
                    &verdict_review.reviewer,
                    task_id,
                ),
            }
        }
        Err(e) => {
            // Check for "already merged" in the error
            let lower = e.to_lowercase();
            if lower.contains("already merged") || lower.contains("pull request is closed") {
                info!(
                    pr_number = pr.number,
                    "PR already finalized — CI success handler acknowledging"
                );
                return VerdictAction::Handled {
                    pre_digest: format_already_merged_pre_digest(
                        &event,
                        pr.number,
                        task.as_ref().map(|t| t.id.as_str()).unwrap_or("none"),
                    ),
                };
            }

            warn!(
                error = %e,
                pr_number = pr.number,
                "CI success structural merge failed"
            );
            VerdictAction::Handled {
                pre_digest: format_error_pre_digest(&event, pr.number, &e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GitHub API helpers
// ---------------------------------------------------------------------------

/// Minimal PR info needed for the handler.
pub(crate) struct PrInfo {
    pub(crate) number: u64,
    pub(crate) head_sha: String,
}

/// Find an open PR whose head branch matches.
pub(crate) async fn find_open_pr(
    repo: &str,
    branch: &str,
    token: &str,
) -> Result<Option<PrInfo>, String> {
    let args = vec![
        "pr",
        "list",
        "--repo",
        repo,
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,headRefOid",
    ];

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh pr list timed out after 60s".to_string())??;

    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(None);
    }

    let prs: Vec<serde_json::Value> = serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse gh pr list output: {e}"))?;

    let first = match prs.first() {
        Some(p) => p,
        None => return Ok(None),
    };

    let number = first["number"]
        .as_u64()
        .ok_or_else(|| "PR number missing from gh pr list output".to_string())?;
    let head_sha = first["headRefOid"]
        .as_str()
        .ok_or_else(|| "headRefOid missing from gh pr list output".to_string())?
        .to_string();

    Ok(Some(PrInfo { number, head_sha }))
}

/// A qualifying review with VERDICT: pass.
struct PassVerdictReview {
    reviewer: String,
    commit_id: String,
}

/// Find the most recent APPROVED review with "VERDICT: pass" in its body.
async fn find_pass_verdict(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Option<PassVerdictReview>, String> {
    let api_path = format!("/repos/{repo}/pulls/{pr_number}/reviews");
    // Note: --paginate is intentionally omitted. gh api --paginate concatenates
    // raw JSON pages as `[...][...]` which is not valid JSON for serde_json.
    // Reviews are returned newest-last and GitHub defaults to 30 per page.
    // PRs with 30+ reviews are rare; if the pass verdict is beyond page 1,
    // the handler will return NoPassVerdict and the LLM handles follow-up.
    let args = vec!["api", &api_path];

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_gh_subprocess(&args, token),
    )
    .await
    .map_err(|_| "gh api reviews timed out after 60s".to_string())??;

    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(None);
    }

    let reviews: Vec<serde_json::Value> = serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse reviews API output: {e}"))?;

    // Find the most recent APPROVED review with "VERDICT: pass" in body.
    // Iterate in reverse to find the latest qualifying review.
    for review in reviews.iter().rev() {
        let state = review["state"].as_str().unwrap_or("");
        if state != "APPROVED" {
            continue;
        }

        let body = review["body"].as_str().unwrap_or("");
        // Reuse the canonical verdict parser to maintain contract parity
        // with verdict_handler (handles VERDICT:pass, VERDICT: pass, etc.)
        if parse_verdict(body) != Verdict::Pass {
            continue;
        }

        let reviewer = review["user"]["login"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let commit_id = review["commit_id"].as_str().unwrap_or("").to_string();

        return Ok(Some(PassVerdictReview {
            reviewer,
            commit_id,
        }));
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

/// Update task metadata with CI success merge state.
async fn update_ci_merge_metadata(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    pr_number: u64,
    pr_url: &str,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "verdict_merge": {
            "state": "merge_initiated",
            "trigger": "check_suite_success",
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

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------

/// Format the pre-digest message for a successful merge action.
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses "initiated" phrasing.
fn format_success_pre_digest(
    event: &CheckSuiteEvent,
    pr_number: u64,
    reviewer: &str,
    task_id: &str,
) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{} (branch: {})\n\
         CI checks all green + VERDICT: pass from @{reviewer} — structural handler acted.\n\n\
         Squash-merge has been initiated for PR #{pr_number} on {} (branch deletion requested).\n\n\
         Task: {task_id}\n\n\
         Do NOT call pr_merge_with_gate — the merge action is already in progress.\n\
         Update the task status to reflect the outcome, then notify the user.\n\
         </ci_success_handler>",
        event.repo, pr_number, event.branch, event.repo
    )
}

/// Format the pre-digest for a PR that was already finalized.
fn format_already_merged_pre_digest(
    event: &CheckSuiteEvent,
    pr_number: u64,
    task_id: &str,
) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{} (branch: {})\n\
         CI checks all green — PR was already finalized before the handler ran.\n\n\
         Task: {task_id}\n\n\
         Do NOT call pr_merge_with_gate — no action needed.\n\
         Update the task status if not already done, then notify the user.\n\
         </ci_success_handler>",
        event.repo, pr_number, event.branch
    )
}

/// Format the pre-digest for a merge error.
fn format_error_pre_digest(event: &CheckSuiteEvent, pr_number: u64, error: &str) -> String {
    format!(
        "<ci_success_handler>\n\
         [GitHub] Check suite success on {}#{} (branch: {})\n\
         CI checks all green — structural handler attempted merge but encountered an error.\n\n\
         Error: {error}\n\n\
         The structural merge handler could not finalize the merge. \
         Investigate the error and decide the next action.\n\
         </ci_success_handler>",
        event.repo, pr_number, event.branch
    )
}

/// Format the enrichment message for a behind-main block.
fn format_behind_main_enrichment(pr_base_sha: &str, current_main_sha: &str) -> String {
    format!(
        "[ci_success_handler] All CI checks passed and VERDICT: pass exists, \
         but the PR is behind main (base: {pr_base_sha}, main HEAD: {current_main_sha}). \
         Rebase the PR onto main before merging.\n\n"
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

    fn sample_event() -> CheckSuiteEvent {
        CheckSuiteEvent {
            repo: "senara-solutions/mika".to_string(),
            branch: "feat/ci-success".to_string(),
        }
    }

    // -- Parser tests --

    #[test]
    fn parse_check_suite_success_valid() {
        let text =
            "[GitHub] Check suite success on senara-solutions/mika (branch: feat/ci-success)";
        let event = parse_check_suite_success(text).unwrap();
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.branch, "feat/ci-success");
    }

    #[test]
    fn parse_check_suite_success_with_trailing_text() {
        let text =
            "[GitHub] Check suite success on org/repo (branch: main)\n\nSome extra context here";
        let event = parse_check_suite_success(text).unwrap();
        assert_eq!(event.repo, "org/repo");
        assert_eq!(event.branch, "main");
    }

    #[test]
    fn parse_check_suite_failure_returns_none() {
        let text =
            "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/ci-success)";
        assert!(parse_check_suite_success(text).is_none());
    }

    #[test]
    fn parse_check_suite_non_matching_returns_none() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#522 by @mika-qa";
        assert!(parse_check_suite_success(text).is_none());
    }

    #[test]
    fn parse_check_suite_empty_returns_none() {
        assert!(parse_check_suite_success("").is_none());
    }

    // -- Pre-digest formatting tests --

    #[test]
    fn success_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, 571, "mika-qa", "task-123");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn success_pre_digest_contains_do_not_call_instruction() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, 571, "mika-qa", "task-123");
        assert!(text.contains("Do NOT call pr_merge_with_gate"));
    }

    #[test]
    fn success_pre_digest_contains_task_id() {
        let event = sample_event();
        let text = format_success_pre_digest(&event, 571, "mika-qa", "task-123");
        assert!(text.contains("task-123"));
    }

    #[test]
    fn already_merged_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_already_merged_pre_digest(&event, 571, "task-123");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn error_pre_digest_contains_error_message() {
        let event = sample_event();
        let text = format_error_pre_digest(&event, 571, "Merge conflicts");
        assert!(text.contains("Merge conflicts"));
        assert!(text.contains("ci_success_handler"));
    }

    #[test]
    fn non_matching_text_returns_passthrough() {
        // This tests the parse path; the full handler test would need mocked subprocesses
        let text = "[GitHub] PR review (approved) on org/repo#42 by @reviewer";
        assert!(parse_check_suite_success(text).is_none());
    }

    // ---- Behind-main enrichment tests (#1577) ----

    #[test]
    fn behind_main_enrichment_contains_both_shas() {
        let pr_base = "abc1234deadbeef";
        let main_head = "def5678cafebabe";
        let text = format_behind_main_enrichment(pr_base, main_head);
        assert!(
            text.contains(pr_base),
            "Enrichment missing pr_base_sha: {text}"
        );
        assert!(
            text.contains(main_head),
            "Enrichment missing current_main_sha: {text}"
        );
    }

    #[test]
    fn behind_main_enrichment_avoids_completion_claim_words() {
        let text = format_behind_main_enrichment("aaa", "bbb");
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Behind-main enrichment contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn behind_main_enrichment_mentions_rebase() {
        let text = format_behind_main_enrichment("aaa", "bbb");
        assert!(
            text.contains("Rebase"),
            "Behind-main enrichment should instruct rebase: {text}"
        );
    }
}
