//! Structural handler for `check_suite.completed(failure|timed_out)` webhook events.
//!
//! Intercepts CI failure events **before** the LLM turn, matches them to open PRs
//! and existing work items, fetches failing-job context, and constructs a pre-digest
//! that instructs the LLM to dispatch `run_claude_pilot` for an autonomous CI fix.
//!
//! Companion to `ci_success_handler` (which handles `check_suite.completed/success`).
//! See issue #594.
//!
//! ## Invariants
//!
//! - Structural handlers MUST return Passthrough for non-matching event types.
//!   Call order is irrelevant — handlers are disjoint on event_type/conclusion.
//!
//! - Loop prevention: `main`/`master` branches are skipped early. Additionally,
//!   `gh pr list --head main --state open` returns empty → NoPr → no-op.
//!
//! - Circuit breaker: `ci_fix_count >= 2` in task metadata triggers escalation
//!   instead of dispatch. The handler increments `ci_fix_count` deterministically
//!   to avoid relying on LLM compliance.
//!
//! - The handler constructs a pre-digest for the LLM — it does NOT directly invoke
//!   `run_claude_pilot`. The dispatch-readiness guard in `executor.rs` is the
//!   authoritative gate for long-running dispatch.
//!
//! ## Forge-gate perimeter (mika#1853)
//!
//! Audited 2026-07-27: this handler has NO `run_gh_merge` callsite — it only builds
//! a pre-digest that suggests a claude-pilot dispatch. No perimeter check is wired in
//! because there is no merge path to gate here. If a future refactor introduces a
//! direct merge callsite in this handler (or its close family), the perimeter fetch +
//! classify pattern from `verdict_handler::handle_pass_verdict` and
//! `ci_success_handler::try_handle_ci_success` (both mika#1853) MUST be applied here
//! too. Grep-anchor before landing: `grep -n 'run_gh_merge' server/*.rs` — every hit
//! must consult `perimeter::classify_pr_files`.

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::task_state::merge_metadata;
use crate::tools::pr_merge_with_gate::{
    CheckClassification, classify_checks, run_gh_checks, run_gh_subprocess,
};

use super::ci_success_handler::{PrInfo, find_open_pr};
use super::verdict_handler::VerdictAction;
use super::webhook_queue::has_active_callback_child;

/// Maximum number of failing job logs to fetch.
const MAX_FAILING_JOBS: usize = 3;

/// Maximum number of lines to keep from each job's log output.
const MAX_LOG_LINES_PER_JOB: usize = 100;

/// Maximum CI fix attempts before escalation.
const MAX_CI_FIX_COUNT: u64 = 2;

/// Parsed fields from a gateway-formatted check_suite failure/timed_out event.
pub(crate) struct CheckSuiteFailureEvent {
    repo: String,
    branch: String,
    conclusion: String,
}

/// Parse the gateway-formatted check_suite failure or timed_out event text.
///
/// Expected format: `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`
/// where conclusion is `failure` or `timed_out`.
pub(crate) fn parse_check_suite_failure(text: &str) -> Option<CheckSuiteFailureEvent> {
    let first_line = text.lines().next()?;

    // Must be a check_suite event with a failure-class conclusion
    if !first_line.starts_with("[GitHub] Check suite ") {
        return None;
    }

    // Extract conclusion (word after "Check suite ")
    let after_prefix = first_line.strip_prefix("[GitHub] Check suite ")?;
    let space_pos = after_prefix.find(' ')?;
    let conclusion = &after_prefix[..space_pos];

    // Only handle failure and timed_out conclusions
    if !matches!(conclusion, "failure" | "timed_out") {
        return None;
    }

    // Expected: "{conclusion} on {repo} (branch: {branch})"
    let after_conclusion = after_prefix.strip_prefix(conclusion)?;
    let after_on = after_conclusion.strip_prefix(" on ")?;

    let branch_marker = " (branch: ";
    let marker_pos = after_on.find(branch_marker)?;
    let repo = &after_on[..marker_pos];

    let after_marker = &after_on[marker_pos + branch_marker.len()..];
    let branch = after_marker.strip_suffix(')')?;

    Some(CheckSuiteFailureEvent {
        repo: repo.to_string(),
        branch: branch.to_string(),
        conclusion: conclusion.to_string(),
    })
}

/// Attempt to handle a CI failure event structurally before the LLM turn.
///
/// Returns `VerdictAction::Handled` when the handler prepared a dispatch pre-digest
/// (or escalation), with structured failure context for the LLM.
/// Returns `VerdictAction::Passthrough` for all other cases (non-matching events,
/// no open PR, no work item, task in wrong status, fix already in-flight).
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ci_failure(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Parse the check_suite failure/timed_out event
    let event = match parse_check_suite_failure(text) {
        Some(e) => e,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    info!(
        repo = %event.repo,
        branch = %event.branch,
        conclusion = %event.conclusion,
        "CI failure handler: evaluating dispatch eligibility"
    );

    // 2. Loop prevention: skip main/master branches
    if matches!(event.branch.as_str(), "main" | "master") {
        info!(
            repo = %event.repo,
            branch = %event.branch,
            "CI failure on default branch — skipping (loop prevention)"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // Require GitHub token for API operations
    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                repo = %event.repo,
                branch = %event.branch,
                "CI failure but no GitHub token available — cannot evaluate"
            );
            return VerdictAction::Passthrough {
                enrichment: Some(
                    "[ci_failure_handler] CI failure detected but no GitHub token configured. \
                     Cannot evaluate dispatch eligibility.\n\n"
                        .to_string(),
                ),
            };
        }
    };

    // 3. Find open PR for this branch
    let pr: PrInfo = match find_open_pr(&event.repo, &event.branch, token).await {
        Ok(Some(pr)) => pr,
        Ok(None) => {
            info!(
                repo = %event.repo,
                branch = %event.branch,
                "CI failure but no open PR for branch — ignoring"
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

    // 4. Find active work item by PR URL, then fall back to branch
    let pr_url = format!("https://github.com/{}/pull/{}", event.repo, pr.number);
    let task = match find_active_task(db, &pr_url, &event.branch).await {
        Some(t) => t,
        None => {
            info!(
                repo = %event.repo,
                branch = %event.branch,
                pr_number = pr.number,
                "CI failure on PR with no matching work item — passing through"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 5. Task must be in_progress
    if task.status != "in_progress" {
        info!(
            task_id = %task.id,
            status = %task.status,
            "CI failure but task not in_progress — passing through"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 6. Check if fix is already in-flight (active callback child)
    if has_active_callback_child(db, &task.id).await {
        info!(
            task_id = %task.id,
            pr_number = pr.number,
            "CI failure but fix already in-flight — enriching passthrough"
        );
        return VerdictAction::Passthrough {
            enrichment: Some(format!(
                "[ci_failure_handler] CI {} on {}#{} (branch: {}). \
                 A fix is already in-flight for task {}. Wait for the callback to finish \
                 before taking further action.\n\n",
                event.conclusion, event.repo, pr.number, event.branch, task.id
            )),
        };
    }

    // 7. Circuit breaker: check ci_fix_count
    let ci_fix_count = read_ci_fix_count(&task.metadata);

    if ci_fix_count >= MAX_CI_FIX_COUNT {
        info!(
            task_id = %task.id,
            ci_fix_count = ci_fix_count,
            "CI failure but fix count >= {} — escalating",
            MAX_CI_FIX_COUNT
        );

        // Log audit event for escalation
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "ci_failure_escalated",
                &format!("task:{}", task.id),
                Some(&task.status),
                None,
                Some(&format!(
                    "trigger=check_suite_{} pr_url={pr_url} ci_fix_count={ci_fix_count}",
                    event.conclusion
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log ci_failure_escalated audit event");
        }

        // Send escalation notification
        if let Some(sender) = message_sender {
            let notification = format!(
                "CI {} on {}#{} (branch: {}) — {} fix attempts exhausted. \
                 Escalating for manual intervention. Task: {}",
                event.conclusion, event.repo, pr.number, event.branch, MAX_CI_FIX_COUNT, task.id
            );
            send_notification(sender, &notification).await;
        }

        return VerdictAction::Handled {
            pre_digest: format_escalation_pre_digest(&event, pr.number, &task.id, ci_fix_count),
        };
    }

    // 8. Fetch failing checks and job logs for context
    //    Done BEFORE incrementing ci_fix_count so transient CI self-healing
    //    doesn't waste a circuit-breaker slot (COR-002).
    let failure_context = fetch_failure_context(pr.number, &event.repo, token).await;

    // 8b. If CI self-healed between the event and our check, abort without
    //     incrementing the counter — no dispatch needed.
    if failure_context.failing_checks.is_empty() && failure_context.error.is_some() {
        info!(
            task_id = %task.id,
            pr_number = pr.number,
            note = failure_context.error.as_deref().unwrap_or("unknown"),
            "CI failure handler: no failing checks at query time — aborting dispatch"
        );
        return VerdictAction::Passthrough {
            enrichment: Some(format!(
                "[ci_failure_handler] CI {} event on {}#{} (branch: {}) but no failing checks \
                 found at query time (possible self-healing). No dispatch needed.\n\n",
                event.conclusion, event.repo, pr.number, event.branch
            )),
        };
    }

    // 9. Increment ci_fix_count deterministically now that we've confirmed failures exist
    let new_count = ci_fix_count + 1;
    if let Err(e) = update_ci_fix_count(db, &task.id, &task.metadata, new_count).await {
        warn!(
            error = %e,
            task_id = %task.id,
            "Failed to increment ci_fix_count — proceeding anyway"
        );
    }

    // 10. Check global dispatch guard (informational — included in pre-digest).
    // CI failures relate to implementation dispatches, so check the 'implement' class (#1001).
    let global_dispatch_busy = match db
        .has_active_callback_tasks_excluding(&task.id, "implement")
        .await
    {
        Ok(Some((parent_id, callback_id, _label))) => {
            info!(
                task_id = %task.id,
                blocking_parent = %parent_id,
                blocking_callback = %callback_id,
                "Global dispatch guard: another task has an active callback"
            );
            Some((parent_id, callback_id))
        }
        Ok(None) => None,
        Err(e) => {
            warn!(error = %e, "Failed to check global dispatch guard");
            None
        }
    };

    // 11. Log audit event
    if let Err(e) = db
        .log_audit_event(
            session_id,
            "ci_failure_handled",
            &format!("task:{}", task.id),
            Some(&task.status),
            None,
            Some(&format!(
                "trigger=check_suite_{} pr_url={pr_url} ci_fix_count={new_count}",
                event.conclusion
            )),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, "Failed to log ci_failure_handled audit event");
    }

    // 12. Send notification
    if let Some(sender) = message_sender {
        let notification = format!(
            "CI {} on {}#{} (branch: {}) — structural handler preparing fix dispatch \
             (attempt {new_count}/{}). Task: {}",
            event.conclusion, event.repo, pr.number, event.branch, MAX_CI_FIX_COUNT, task.id
        );
        send_notification(sender, &notification).await;
    }

    info!(
        task_id = %task.id,
        pr_number = pr.number,
        repo = %event.repo,
        conclusion = %event.conclusion,
        ci_fix_count = new_count,
        "CI failure handler: dispatch pre-digest prepared"
    );

    VerdictAction::Handled {
        pre_digest: format_dispatch_pre_digest(
            &event,
            pr.number,
            &task.id,
            new_count,
            &failure_context,
            global_dispatch_busy.as_ref(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Task lookup helpers
// ---------------------------------------------------------------------------

/// Find an active task by PR URL first, then fall back to branch-based lookup.
async fn find_active_task(
    db: &AsyncDatabase,
    pr_url: &str,
    branch: &str,
) -> Option<crate::db::Task> {
    // Try PR URL first (more precise)
    match db.find_active_task_by_pr_url(pr_url).await {
        Ok(Some(task)) => return Some(task),
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, pr_url = %pr_url, "Failed to look up task by PR URL");
        }
    }

    // Fall back to branch-based lookup
    match db.find_active_task_by_branch(branch).await {
        Ok(task) => task,
        Err(e) => {
            warn!(error = %e, branch = %branch, "Failed to look up task by branch");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

/// Read `ci_fix_count` from task metadata JSON, defaulting to 0.
fn read_ci_fix_count(metadata: &Option<String>) -> u64 {
    metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("ci_fix_count")?.as_u64())
        .unwrap_or(0)
}

/// Increment `ci_fix_count` in task metadata.
async fn update_ci_fix_count(
    db: &AsyncDatabase,
    task_id: &str,
    existing_metadata: &Option<String>,
    new_count: u64,
) -> Result<()> {
    let mut base = existing_metadata
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| json!({}));

    let incoming = json!({
        "ci_fix_count": new_count,
        "ci_failure": {
            "last_handled_at": crate::timestamp::now(),
        }
    });

    merge_metadata(&mut base, &incoming);
    let merged_str = serde_json::to_string(&base)?;
    db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Failure context gathering
// ---------------------------------------------------------------------------

/// Fetched failure context from CI checks and job logs.
struct FailureContext {
    /// Failing check names and their states.
    failing_checks: Vec<(String, String)>,
    /// Job log excerpts (job_name, truncated log).
    job_logs: Vec<(String, String)>,
    /// Error message if context gathering partially failed.
    error: Option<String>,
}

/// Fetch failing checks and job logs for a PR.
async fn fetch_failure_context(pr_number: u64, repo: &str, token: &str) -> FailureContext {
    let mut ctx = FailureContext {
        failing_checks: Vec::new(),
        job_logs: Vec::new(),
        error: None,
    };

    // Fetch checks with timeout
    let checks = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_gh_checks(pr_number, repo, token),
    )
    .await
    {
        Ok(Ok(checks)) => checks,
        Ok(Err(e)) => {
            warn!(error = %e, pr_number, "Failed to fetch CI checks for failure context");
            ctx.error = Some(format!("Failed to fetch checks: {e}"));
            return ctx;
        }
        Err(_) => {
            warn!(pr_number, "CI check fetch timed out for failure context");
            ctx.error = Some("Check fetch timed out after 30s".to_string());
            return ctx;
        }
    };

    let classification = classify_checks(&checks);
    if classification != CheckClassification::HasFailures {
        // CI might have recovered between the event and our check
        ctx.error = Some(format!(
            "No failing checks found at query time (classification: {:?})",
            classification
        ));
        return ctx;
    }

    // Identify failing checks
    let failing: Vec<_> = checks
        .iter()
        .filter(|c| matches!(c.bucket.as_str(), "fail" | "cancel"))
        .collect();

    for check in &failing {
        ctx.failing_checks
            .push((check.name.clone(), check.state.clone()));
    }

    // Fetch job logs for up to MAX_FAILING_JOBS
    for check in failing.iter().take(MAX_FAILING_JOBS) {
        if let Some(ref link) = check.link
            && let Some(log) = fetch_job_log(link, repo, token).await
        {
            ctx.job_logs.push((check.name.clone(), log));
        }
    }

    ctx
}

/// Parse a GitHub Actions check link URL into (run_id, job_id).
///
/// Expected format: `https://github.com/{owner}/{repo}/actions/runs/{run_id}/job/{job_id}`
fn parse_check_link(url: &str) -> Option<(&str, &str)> {
    // Split on "/job/" to separate run path from job_id
    let (before_job, job_id) = url.rsplit_once("/job/")?;
    // Extract run_id: last segment before "/job/"
    let run_id = before_job.rsplit('/').next()?;
    if run_id.is_empty() || job_id.is_empty() {
        return None;
    }
    Some((run_id, job_id))
}

/// Fetch and truncate a failing job's log output.
///
/// Extracts the run ID and job name from the check link URL, then runs
/// `gh run view <run_id> --repo <repo> --job <job_name> --log-failed`.
async fn fetch_job_log(check_link: &str, repo: &str, token: &str) -> Option<String> {
    // check_link format: https://github.com/{owner}/{repo}/actions/runs/{run_id}/job/{job_id}
    // gh run view requires: gh run view <run_id> --repo <repo> --job <job_id> --log-failed
    let (run_id, job_id) = parse_check_link(check_link)?;

    let args = vec![
        "run",
        "view",
        run_id,
        "--repo",
        repo,
        "--job",
        job_id,
        "--log-failed",
    ];

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_gh_subprocess(&args, token),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let lines: Vec<&str> = output.lines().collect();
            let truncated = if lines.len() > MAX_LOG_LINES_PER_JOB {
                let skip = lines.len() - MAX_LOG_LINES_PER_JOB;
                format!(
                    "[... {} lines truncated ...]\n{}",
                    skip,
                    lines[skip..].join("\n")
                )
            } else {
                output
            };
            Some(truncated)
        }
        Ok(Err(e)) => {
            warn!(job_id, error = %e, "Failed to fetch job log");
            None
        }
        Err(_) => {
            warn!(job_id, "Job log fetch timed out after 30s");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Notification helper
// ---------------------------------------------------------------------------

async fn send_notification(sender: &Arc<dyn MessageSender>, message: &str) {
    match sender.send(message).await {
        Ok(crate::messaging::SendOutcome::Delivered) => {}
        Ok(crate::messaging::SendOutcome::Failed { reason }) => {
            warn!(reason = %reason, "CI failure handler notification delivery failed");
        }
        Ok(crate::messaging::SendOutcome::NoChannel) => {
            warn!("CI failure handler notification skipped — no reply channel (chat_id=0)");
        }
        Err(e) => {
            warn!(error = %e, "Failed to send CI failure handler notification");
        }
    }
}

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------

/// Format the pre-digest message for a dispatch instruction.
///
/// IMPORTANT: Avoids completion-claim guard trigger words (merged, deployed,
/// completed, complete, shipped). Uses present-tense phrasing.
fn format_dispatch_pre_digest(
    event: &CheckSuiteFailureEvent,
    pr_number: u64,
    task_id: &str,
    ci_fix_count: u64,
    failure_context: &FailureContext,
    global_dispatch_busy: Option<&(String, String)>,
) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "<ci_failure_handler>\n\
         [GitHub] Check suite {} on {}#{} (branch: {})\n\
         CI failure detected — structural handler has gathered context for dispatch.",
        event.conclusion, event.repo, pr_number, event.branch
    ));

    parts.push(format!("\nTask: {task_id}"));
    parts.push(format!("CI fix attempt: {ci_fix_count}/{MAX_CI_FIX_COUNT}"));

    // Failing checks
    if !failure_context.failing_checks.is_empty() {
        parts.push("\nFailing checks:".to_string());
        for (name, state) in &failure_context.failing_checks {
            parts.push(format!("  - {name} ({state})"));
        }
    }

    // Job logs
    if !failure_context.job_logs.is_empty() {
        parts.push("\nJob log excerpts:".to_string());
        for (name, log) in &failure_context.job_logs {
            parts.push(format!("\n--- {name} ---\n{log}"));
        }
    }

    // Error if context gathering had issues
    if let Some(ref err) = failure_context.error {
        parts.push(format!("\nContext gathering note: {err}"));
    }

    // Global dispatch status
    if let Some((parent_id, callback_id)) = global_dispatch_busy {
        parts.push(format!(
            "\nWARNING: Another task ({parent_id}) has an active callback ({callback_id}). \
             run_claude_pilot dispatch will be rejected by the global single-session guard. \
             Wait for that callback to finish, or notify the user about the CI failure."
        ));
    }

    parts.push(
        "\nAction required: dispatch run_claude_pilot with skill: \"dev-pilot\" to fix the CI failure on this branch. \
         Do NOT re-increment ci_fix_count — the structural handler already updated it.\n\
         </ci_failure_handler>"
            .to_string(),
    );

    parts.join("\n")
}

/// Format the pre-digest for escalation (fix count exhausted).
fn format_escalation_pre_digest(
    event: &CheckSuiteFailureEvent,
    pr_number: u64,
    task_id: &str,
    ci_fix_count: u64,
) -> String {
    format!(
        "<ci_failure_handler>\n\
         [GitHub] Check suite {} on {}#{} (branch: {})\n\
         CI failure detected — {ci_fix_count} fix attempts already made (limit: {MAX_CI_FIX_COUNT}).\n\n\
         Task: {task_id}\n\n\
         Do NOT dispatch run_claude_pilot — the retry budget is exhausted.\n\
         Notify the user about the persistent CI failure and ask for manual intervention.\n\
         </ci_failure_handler>",
        event.conclusion, event.repo, pr_number, event.branch
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

    fn sample_event() -> CheckSuiteFailureEvent {
        CheckSuiteFailureEvent {
            repo: "senara-solutions/mika".to_string(),
            branch: "feat/ci-fix".to_string(),
            conclusion: "failure".to_string(),
        }
    }

    fn sample_failure_context() -> FailureContext {
        FailureContext {
            failing_checks: vec![
                ("CI".to_string(), "failure".to_string()),
                ("Lint".to_string(), "failure".to_string()),
            ],
            job_logs: vec![(
                "CI".to_string(),
                "error[E0308]: mismatched types".to_string(),
            )],
            error: None,
        }
    }

    // -- Parser tests --

    #[test]
    fn parse_check_suite_failure_valid() {
        let text = "[GitHub] Check suite failure on senara-solutions/mika (branch: feat/ci-fix)";
        let event = parse_check_suite_failure(text).unwrap();
        assert_eq!(event.repo, "senara-solutions/mika");
        assert_eq!(event.branch, "feat/ci-fix");
        assert_eq!(event.conclusion, "failure");
    }

    #[test]
    fn parse_check_suite_timed_out_valid() {
        let text = "[GitHub] Check suite timed_out on org/repo (branch: fix/timeout-bug)";
        let event = parse_check_suite_failure(text).unwrap();
        assert_eq!(event.repo, "org/repo");
        assert_eq!(event.branch, "fix/timeout-bug");
        assert_eq!(event.conclusion, "timed_out");
    }

    #[test]
    fn parse_check_suite_failure_with_trailing_text() {
        let text = "[GitHub] Check suite failure on org/repo (branch: main)\n\nSome extra context";
        let event = parse_check_suite_failure(text).unwrap();
        assert_eq!(event.repo, "org/repo");
        assert_eq!(event.branch, "main");
    }

    #[test]
    fn parse_check_suite_success_returns_none() {
        let text = "[GitHub] Check suite success on senara-solutions/mika (branch: feat/ci-fix)";
        assert!(parse_check_suite_failure(text).is_none());
    }

    #[test]
    fn parse_check_suite_non_matching_event_returns_none() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#522 by @mika-qa";
        assert!(parse_check_suite_failure(text).is_none());
    }

    #[test]
    fn parse_check_suite_empty_returns_none() {
        assert!(parse_check_suite_failure("").is_none());
    }

    #[test]
    fn parse_check_suite_malformed_missing_branch_returns_none() {
        let text = "[GitHub] Check suite failure on repo";
        assert!(parse_check_suite_failure(text).is_none());
    }

    #[test]
    fn parse_check_suite_unknown_conclusion_returns_none() {
        let text = "[GitHub] Check suite cancelled on org/repo (branch: feat/x)";
        assert!(parse_check_suite_failure(text).is_none());
    }

    // -- Check link parser tests --

    #[test]
    fn parse_check_link_valid() {
        let url = "https://github.com/org/repo/actions/runs/12345/job/67890";
        let (run_id, job_id) = parse_check_link(url).unwrap();
        assert_eq!(run_id, "12345");
        assert_eq!(job_id, "67890");
    }

    #[test]
    fn parse_check_link_no_job_segment() {
        let url = "https://github.com/org/repo/actions/runs/12345";
        assert!(parse_check_link(url).is_none());
    }

    #[test]
    fn parse_check_link_empty_returns_none() {
        assert!(parse_check_link("").is_none());
    }

    // -- Metadata helpers --

    #[test]
    fn read_ci_fix_count_from_metadata() {
        let meta = Some(r#"{"ci_fix_count": 2}"#.to_string());
        assert_eq!(read_ci_fix_count(&meta), 2);
    }

    #[test]
    fn read_ci_fix_count_missing() {
        let meta = Some(r#"{"other": "value"}"#.to_string());
        assert_eq!(read_ci_fix_count(&meta), 0);
    }

    #[test]
    fn read_ci_fix_count_none_metadata() {
        assert_eq!(read_ci_fix_count(&None), 0);
    }

    #[test]
    fn read_ci_fix_count_invalid_json() {
        let meta = Some("not json".to_string());
        assert_eq!(read_ci_fix_count(&meta), 0);
    }

    // -- Pre-digest formatting tests --

    #[test]
    fn dispatch_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, None);
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn dispatch_pre_digest_contains_xml_tags() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, None);
        assert!(text.contains("<ci_failure_handler>"));
        assert!(text.contains("</ci_failure_handler>"));
    }

    #[test]
    fn dispatch_pre_digest_contains_task_id() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, None);
        assert!(text.contains("task-123"));
    }

    #[test]
    fn dispatch_pre_digest_contains_no_reincrement_instruction() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, None);
        assert!(text.contains("Do NOT re-increment ci_fix_count"));
    }

    #[test]
    fn dispatch_pre_digest_contains_failure_context() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, None);
        assert!(text.contains("CI (failure)"));
        assert!(text.contains("Lint (failure)"));
        assert!(text.contains("mismatched types"));
    }

    #[test]
    fn dispatch_pre_digest_includes_global_dispatch_warning() {
        let event = sample_event();
        let ctx = sample_failure_context();
        let busy = ("other-task".to_string(), "callback-123".to_string());
        let text = format_dispatch_pre_digest(&event, 592, "task-123", 1, &ctx, Some(&busy));
        assert!(text.contains("other-task"));
        assert!(text.contains("callback-123"));
        assert!(text.contains("global single-session guard"));
    }

    #[test]
    fn escalation_pre_digest_avoids_completion_claim_words() {
        let event = sample_event();
        let text = format_escalation_pre_digest(&event, 592, "task-123", 2);
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "Escalation pre-digest contains completion-claim trigger word: {text}"
        );
    }

    #[test]
    fn escalation_pre_digest_contains_no_dispatch_instruction() {
        let event = sample_event();
        let text = format_escalation_pre_digest(&event, 592, "task-123", 2);
        assert!(text.contains("Do NOT dispatch run_claude_pilot"));
        assert!(text.contains("retry budget is exhausted"));
    }

    #[test]
    fn escalation_pre_digest_contains_xml_tags() {
        let event = sample_event();
        let text = format_escalation_pre_digest(&event, 592, "task-123", 2);
        assert!(text.contains("<ci_failure_handler>"));
        assert!(text.contains("</ci_failure_handler>"));
    }

    // -- Passthrough enrichment formatting --

    #[test]
    fn in_flight_enrichment_avoids_completion_claim_words() {
        // Simulate the enrichment text from the in-flight branch
        let text = format!(
            "[ci_failure_handler] CI {} on {}#{} (branch: {}). \
             A fix is already in-flight for task {}. Wait for the callback to finish \
             before taking further action.\n\n",
            "failure", "senara-solutions/mika", 592, "feat/ci-fix", "task-123"
        );
        assert!(
            !COMPLETION_CLAIM_RE.is_match(&text),
            "In-flight enrichment contains completion-claim trigger word: {text}"
        );
    }
}
