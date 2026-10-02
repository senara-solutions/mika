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
//! ## Surface-for-adoption (mika#1745)
//!
//! Step 4's `None` arm — a CI failure on an open PR that no mika task matches —
//! used to return `Passthrough { enrichment: None }`, i.e. hand the LLM the raw
//! webhook text under a prompt whose decision table says *"ignore: this CI
//! failure is not from our work"*. That silence is the defect mika#1745 was
//! filed against, and it is written **here** rather than in the prompt because
//! `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
//! (mika#2120: nine recurrences under prompt enforcement against zero when the
//! fact is posed by code) says the prompt half does not hold alone.
//!
//! [`surface_for_adoption`] replaces that arm. It is **additive**: it creates no
//! task, signals no process, changes no status, blocks no dispatch — AC3 is held
//! by `Passthrough`, never `Handled`, because `Handled` would tell the LLM an
//! action had been taken. Three surfaces, and only the first two hold whatever
//! the model does with them: an `audit_events` row, an operator notification,
//! and an `enrichment` the LLM reads.
//!
//! Detection is unconditional; only the notification is gated
//! (`MIKA_SURFACE_FOR_ADOPTION`, motif mika#2249/#2272). The row and the log line
//! **are** the measurement, and they have to stay readable exactly when an
//! operator cut the noise.
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

/// The audit `tool_name` and the log event name of a surface-for-adoption signal
/// (mika#1745).
///
/// **SOLE WRITER**, in the log and in `audit_events`, held by
/// `canonical_tokens::tests::mika1745_the_surface_name_has_a_single_writer`. That
/// property is what makes `SELECT count(*) … GROUP BY target_key` an exact count
/// rather than a number two sites can disagree about — and the count is the
/// explicit precondition of the auto-adoption decision AC3 defers ("until we
/// have enough n").
const SURFACE_FOR_ADOPTION_TOOL: &str = "surface_for_adoption";

/// Dedup window for a surface-for-adoption signal, keyed `(repo, PR, head_sha)`.
///
/// One push produces up to **8** `check_suite.completed` events, one per
/// workflow — measured and documented by mika#1869, which had to close the same
/// class on the success side. Without dedup a single CI failure with no task
/// would produce up to eight Telegram notifications for one fact, and a
/// notification that arrives in bursts is a notification one ends up muzzling.
/// So the dedup is a viability condition, not a refinement.
///
/// One hour covers the fan-out of a push (seconds) and a GitHub redelivery
/// (minutes) by a wide margin, and a second CI failure on the **same**
/// `head_sha` an hour later is not a new fact.
const SURFACE_DEDUP_WINDOW_SECS: i64 = 3600;

/// Log event name for a surface-for-adoption the handler **withheld**.
///
/// Deliberately distinct from [`SURFACE_FOR_ADOPTION_TOOL`], which is SOLE
/// WRITER: a skip is not a signal, and folding the two would make the two
/// operator populations impossible to count apart — the motif
/// `phantom_aged_out` / `phantom_sweep_spared` (mika#2156).
const SURFACE_SKIPPED_EVENT: &str = "surface_for_adoption_skipped";

/// The repository is outside `DISPATCHABLE_REPOS` — AC4, a legitimate silence.
const SURFACE_SKIP_REPO_NOT_DISPATCHABLE: &str = "repo_not_dispatchable";

/// No readable `head_sha`, so no per-push dedup key could be derived.
const SURFACE_SKIP_UNREADABLE_HEAD_SHA: &str = "unreadable_head_sha";

// The frozen list of the two motives above lives in `tests` as
// `ALL_SURFACE_SKIP_MOTIVES` — see its comment for why it is not here.

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
///
/// `settings` is read for one thing only: whether a surface-for-adoption signal
/// may reach the operator (mika#1745). It is resolved inside
/// [`surface_for_adoption`], **after** the dispatchable-repo term, so a
/// misconfigured value is named on the population that would have been notified
/// rather than on every CI failure the fleet sees.
#[allow(clippy::too_many_arguments)]
pub async fn try_handle_ci_failure(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    settings: &mika_common::config::Settings,
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
            // mika#1745 — the only site that surfaces for adoption, and the
            // invariant that makes AC4 a property of the call graph: a PR that
            // HAS a task cannot reach it. Pinned by
            // `tests::mika1745_t3_the_only_call_site_is_the_no_task_arm`.
            return surface_for_adoption(
                db,
                &event,
                &pr,
                &pr_url,
                message_sender,
                session_id,
                trace_id,
                settings,
            )
            .await;
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
        Ok(Some(blocking)) => {
            info!(
                task_id = %task.id,
                blocking_parent = %blocking.parent_task_id,
                blocking_callback = %blocking.callback_task_id,
                blocking_dispatcher_source =
                    blocking.dispatcher_source.as_deref().unwrap_or("unknown"),
                "Global dispatch guard: another task has an active callback"
            );
            Some((blocking.parent_task_id, blocking.callback_task_id))
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
// Surface-for-adoption (mika#1745)
// ---------------------------------------------------------------------------

/// Surface a CI failure that no mika task matches, so the operator can decide
/// whether to adopt it — and stay silent when the repository is out of the
/// loop's reach (mika#1745).
///
/// # Every term is fail-safe towards today's silence
///
/// A missed surface costs an event a human does not see: visible, recoverable,
/// and the pre-mika#1745 behaviour. A spurious surface costs one log line. So any
/// unreadable information (a repository that is not dispatchable, an unreadable
/// `head_sha`, a database that will not answer the dedup query) falls back on
/// what this arm did before. Neither error is destructive, which is precisely why
/// this mechanism can ship armed.
///
/// # The ownership list is `DISPATCHABLE_REPOS`, and that is a decision
///
/// Two lists exist and they are not the same question. The gateway's
/// `INTERNAL_REPOS` decides which repositories **route** to mika-dev — it carries
/// `claude-pilot-py` and `wizzard`. `DISPATCHABLE_REPOS` decides where the loop
/// may **dispatch**, and the 2026-08-29 operator decision keeps those two out of
/// it. AC1's "ownership list" is the second one, because it answers the question
/// the surface actually asks: *is the adoption being proposed executable?*
/// Surfacing `claude-pilot-py` would propose an action the mika#2046 gate refuses
/// structurally — so that repository belongs to AC4 (silent-stop), not AC1. A
/// third list would be the programmed divergence this repo has already paid for
/// twice (dispatch seats mika#2092, `DISPATCHABLE_REPOS` ↔ `labels.yml`).
///
/// # Dedup is fail-open
///
/// An unreadable `audit_events` lets the surface through: a duplicate
/// notification costs a line, a lost surface reopens the defect. Deliberately the
/// inverse of `wip_rescue`'s fail-closed read of the same primitive, where a
/// replay cost the whole queue.
#[allow(clippy::too_many_arguments)]
async fn surface_for_adoption(
    db: &AsyncDatabase,
    event: &CheckSuiteFailureEvent,
    pr: &PrInfo,
    pr_url: &str,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
    settings: &mika_common::config::Settings,
) -> VerdictAction {
    // Term 1 (AC4) — the repository is out of the loop's reach. Byte-for-byte
    // the pre-mika#1745 behaviour, said rather than merely done: a silence
    // nobody can attribute is what this ticket exists to end, and that includes
    // the legitimate silences.
    if !crate::webhook_dispatch::is_dispatchable_repo(&event.repo) {
        info!(
            event = SURFACE_SKIPPED_EVENT,
            reason = SURFACE_SKIP_REPO_NOT_DISPATCHABLE,
            repo = %event.repo,
            branch = %event.branch,
            pr_number = pr.number,
            "CI failure on PR with no matching work item, in a repository the loop \
             may not dispatch into — passing through (mika#1745 AC4)"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // Term 2 — no dedup key, no surface. `head_sha` is what makes the key
    // per-push rather than per-PR; without it the burst of up to 8 workflow
    // events could not be collapsed, and notifying eight times for one fact is
    // how a signal gets muzzled.
    let head_sha = pr.head_sha.trim();
    if head_sha.is_empty() {
        info!(
            event = SURFACE_SKIPPED_EVENT,
            reason = SURFACE_SKIP_UNREADABLE_HEAD_SHA,
            repo = %event.repo,
            branch = %event.branch,
            pr_number = pr.number,
            "CI failure with no matching work item but no readable head SHA — \
             cannot key the dedup, passing through (mika#1745)"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    let dedup_key = format!("pr:{}#{}@{}", event.repo, pr.number, head_sha);
    let since = crate::timestamp::now_minus(chrono::Duration::seconds(SURFACE_DEDUP_WINDOW_SECS));
    let recent = db
        .count_recent_audit_events_for_target(SURFACE_FOR_ADOPTION_TOOL, &dedup_key, &since)
        .await
        .inspect_err(|e| {
            warn!(
                error = %e,
                repo = %event.repo,
                pr_number = pr.number,
                "surface-for-adoption dedup query failed — surfacing anyway (fail-open)"
            );
        })
        .map_err(|_| ());
    let already_surfaced = is_already_surfaced(recent);

    let enrichment = format_surface_enrichment(event, pr.number);

    // Term 3 — a duplicate of the same push. The LLM still gets the fact (the
    // enrichment is what stops it reading the bare webhook under a prompt that
    // says to ignore it); what is withheld is the second notification and the
    // second audit row, which would make the count inexact.
    if already_surfaced {
        info!(
            event = SURFACE_FOR_ADOPTION_TOOL,
            repo = %event.repo,
            branch = %event.branch,
            pr_number = pr.number,
            head_sha = %head_sha,
            dedup_skipped = true,
            notified = false,
            "surface-for-adoption already recorded for this head SHA within the \
             window — enriching without a second notification (mika#1745)"
        );
        return VerdictAction::Passthrough {
            enrichment: Some(enrichment),
        };
    }

    // Detection is unconditional: the row and the log line are the measurement.
    if let Err(e) = db
        .log_audit_event(
            session_id,
            SURFACE_FOR_ADOPTION_TOOL,
            &dedup_key,
            None,
            Some(&event.conclusion),
            Some(&format!(
                "trigger=check_suite_{} repo={} branch={} pr_url={pr_url}",
                event.conclusion, event.repo, event.branch
            )),
            Some(trace_id),
        )
        .await
    {
        // A named event rather than the generic warn line: the count in
        // `audit_events` is what sizes the auto-adoption decision, so a row that
        // did not land makes the `GROUP BY` undercount, and that is worth being
        // able to grep on its own.
        warn!(
            event = "surface_for_adoption_audit_failed",
            error = %e,
            repo = %event.repo,
            pr_number = pr.number,
            "failed to write the surface_for_adoption audit row (continuing)"
        );
    }

    // Only the disposition is gated.
    let armed = settings.surface_for_adoption_notifications_armed();
    let mut notified = false;
    if armed && let Some(sender) = message_sender {
        send_notification(
            sender,
            &format_surface_notification(event, pr.number, pr_url),
        )
        .await;
        notified = true;
    }

    info!(
        event = SURFACE_FOR_ADOPTION_TOOL,
        repo = %event.repo,
        branch = %event.branch,
        pr_number = pr.number,
        head_sha = %head_sha,
        dedup_skipped = false,
        notified = notified,
        notifications_armed = armed,
        "CI failure on a dispatchable repo with no matching work item — surfaced \
         for adoption (mika#1745). No task created, no dispatch triggered."
    );

    VerdictAction::Passthrough {
        enrichment: Some(enrichment),
    }
}

/// Has this `(repo, PR, head_sha)` already been surfaced inside the window?
///
/// Takes the **result** rather than a count, so the fail-open half is a property
/// of a pure function instead of a branch of an async body nothing can reach in a
/// test: an unreadable `audit_events` surfaces anyway, because a duplicate
/// notification costs one line while a lost surface reopens the defect. Modelled
/// on `ci_success_handler::is_duplicate_processed`, which is the same decision on
/// the success side — with the `Result` added, which is the half that one could
/// not assert.
///
/// Deliberately the inverse of `wip_rescue`'s fail-closed read of the same
/// primitive: there a replay cost the whole queue, here it costs a line.
fn is_already_surfaced(recent_count: Result<i64, ()>) -> bool {
    matches!(recent_count, Ok(count) if count >= 1)
}

/// The enrichment the LLM reads, carrying AC2's four elements (a)–(d).
///
/// English rather than the plan's French, for local coherence: every other
/// enrichment in this file and the `self-dev-webhook-ci` prompt this composes
/// with are English, and a French instruction inside an English context is one
/// the model reads worse, not better. The substance is the plan's, verbatim in
/// its four parts.
///
/// **The last line is load-bearing.** Without it a model reading "proposed
/// adoption" under a prompt that tells it to ignore the event is left in front of
/// a contradiction, and that is the kind of gap this house has measured as
/// expensive. It closes AC3 on the intent side: nothing here asks the model to
/// act.
///
/// Avoids the completion-claim guard's trigger words (`merged`, `deployed`,
/// `complete`/`completed`, `shipped`) like every other message this file
/// composes — pinned by a test below.
fn format_surface_enrichment(event: &CheckSuiteFailureEvent, pr_number: u64) -> String {
    format!(
        "[surface_for_adoption] CI {} on {}#{} (branch: {}).\n\
         No mika task matches this PR — this work did not go through the self-dev \
         pipeline. The repository IS one the loop may dispatch into.\n\
         Proposed adoption: create a task from this event and engage.\n\
         Do NOT adopt on your own initiative — the operator ratifies adoption \
         case by case.\n\n",
        event.conclusion, event.repo, pr_number, event.branch
    )
}

/// The operator notification. Same shape as its two siblings in this file.
///
/// It **names the lifting gesture**: a signal that does not say what to do with
/// it is a signal one learns to ignore.
fn format_surface_notification(
    event: &CheckSuiteFailureEvent,
    pr_number: u64,
    pr_url: &str,
) -> String {
    format!(
        "Unadopted work: CI {} on {}#{} (branch: {}). \
         No mika task — the loop did not dispatch it. {pr_url}\n\
         To adopt: put `ready` on the linked issue, or dispatch by hand.",
        event.conclusion, event.repo, pr_number, event.branch
    )
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
///
/// **This site COLLECTS, it does not decide — and that is why mika#2617 changed
/// it without touching a line of it.** The four consumers that decide through
/// `classify_checks` inherit U1's widening for free; this one filters
/// `fail|cancel` by hand, so its population silently grew from "the red
/// *required* checks" to "every red check of the head".
///
/// Two gains and one cost, named here rather than left to be rediscovered:
///
/// 1. A non-required lint that is red now enters the repair context, where the
///    model used to receive a context mute about the very failure it had to fix.
/// 2. The `classification != HasFailures` arm ("CI might have recovered between
///    the event and our check") becomes rarer — it used to fire whenever the red
///    check was not a required one.
/// 3. **Cost.** `MAX_FAILING_JOBS` bounds how many job logs are fetched, in the
///    order `gh` returns them. A real build failure sitting behind several red
///    lints therefore **loses its log**: the repair context degrades on exactly
///    the case where it matters most.
///
/// Prioritising the list is deliberately **not** done: ranking checks by
/// importance would reintroduce a notion of "a check that counts more", which is
/// the divergence mika#2617 exists to close. Raising the bound trades a truncated
/// context for a larger prompt with no measurement asking for it. The follow-up
/// is conditioned on a measurement showing a repair context whose useful log was
/// missing because lints occupied the `MAX_FAILING_JOBS` slots.
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

    // ───────────────────────────────────────────────────────────────────
    // mika#1745 — surface-for-adoption
    //
    // Each term of the predicate is exercised on its own. A conjunction of
    // fail-safe terms is not proven by neutralising them all at once — the
    // mika#2277 lesson, whose two false positives read "nominal" on first
    // inspection because one age was reported for a disposition crossing three.
    // ───────────────────────────────────────────────────────────────────

    /// The skip motives, as a **wire format**.
    ///
    /// They are published as operator surfaces
    /// (`docs/architecture/mika-dev-work-assignment.md` § 7 and the root
    /// `CLAUDE.md`), and an operator greps and groups on them — so two spellings
    /// of one motive would split a population without saying so. Same reasoning
    /// as `worktree_reaper::ALL_REFUSAL_REASONS`.
    ///
    /// **In `tests` rather than beside the constants it lists, unlike its
    /// `worktree_reaper` model, and for two reasons.** That list has production
    /// consumers (the audit writer reads it); this one has none — its whole role
    /// is to be the frozen list, so in production it would be dead code, and a
    /// constant nothing reads is a constant nobody keeps current. And a
    /// `#[cfg(test)]` **in the middle of the file** is not free here: several
    /// source scans in this repo — `canonical_tokens::production_sources` and
    /// [`mika1745_t3_the_only_call_site_is_the_no_task_arm`] among them —
    /// truncate a file at its first textual `#[cfg(test)]`, so one placed above
    /// line 290 hides the production call site from T3. Measured: T3 went from 1
    /// call site to 0 on exactly that edit.
    ///
    /// What the move costs is the exhaustiveness a production list gets for free,
    /// which [`mika1745_every_declared_skip_motive_is_in_the_wire_format_list`]
    /// buys back by scanning the declarations through `include_str!` — which sees
    /// the production half regardless of where this list sits.
    const ALL_SURFACE_SKIP_MOTIVES: &[&str] = &[
        SURFACE_SKIP_REPO_NOT_DISPATCHABLE,
        SURFACE_SKIP_UNREADABLE_HEAD_SHA,
    ];

    const DISPATCHABLE: &str = "senara-solutions/mika";
    const NOT_DISPATCHABLE: &str = "senara-solutions/claude-pilot-py";
    const SURFACE_SESSION: &str = "surface-1745-session";

    fn surface_event(repo: &str) -> CheckSuiteFailureEvent {
        CheckSuiteFailureEvent {
            repo: repo.to_string(),
            branch: "feat/1745/gate-surface".to_string(),
            conclusion: "failure".to_string(),
        }
    }

    fn pr_info(head_sha: &str) -> PrInfo {
        PrInfo {
            number: 1745,
            head_sha: head_sha.to_string(),
        }
    }

    /// Agent `"mika"` rather than `"mika-dev"`: `sessions.agent_id` carries a
    /// foreign key, and only the default agent exists in a fresh in-memory
    /// database. Same choice, for the same reason, as
    /// `ci_success_handler`'s `test_db`.
    async fn surface_db() -> AsyncDatabase {
        let db = crate::db::Database::open_in_memory().expect("in-memory db");
        db.create_session(SURFACE_SESSION, "mika", "github")
            .expect("create session");
        AsyncDatabase::new(db)
    }

    /// A sender that records what it was asked to deliver. `NoopSender` cannot
    /// serve here: T6 has to tell "the notification was withheld" from "no
    /// notification was attempted", and a sender that answers `Delivered` to
    /// everything makes those two indistinguishable.
    #[derive(Default)]
    struct RecordingSender {
        sent: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingSender {
        fn messages(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl MessageSender for RecordingSender {
        async fn send(&self, text: &str) -> Result<crate::messaging::SendOutcome> {
            self.sent.lock().unwrap().push(text.to_string());
            Ok(crate::messaging::SendOutcome::Delivered)
        }
    }

    async fn audit_count(db: &AsyncDatabase, key: &str) -> i64 {
        let since =
            crate::timestamp::now_minus(chrono::Duration::seconds(SURFACE_DEDUP_WINDOW_SECS * 2));
        db.count_recent_audit_events_for_target(SURFACE_FOR_ADOPTION_TOOL, key, &since)
            .await
            .expect("audit count readable")
    }

    fn armed_settings() -> mika_common::config::Settings {
        mika_common::config::Settings::test_defaults()
    }

    fn disarmed_settings() -> mika_common::config::Settings {
        let mut s = mika_common::config::Settings::test_defaults();
        s.surface_for_adoption = Some("0".to_string());
        s
    }

    /// T1 — the founding population: a dispatchable repo, an open PR, no task.
    #[tokio::test]
    async fn mika1745_t1_a_dispatchable_repo_without_a_task_is_surfaced() {
        let db = surface_db().await;
        let recorder = Arc::new(RecordingSender::default());
        let sender: Arc<dyn MessageSender> = recorder.clone();

        let action = surface_for_adoption(
            &db,
            &surface_event(DISPATCHABLE),
            &pr_info("abc123"),
            "https://github.com/senara-solutions/mika/pull/1745",
            Some(&sender),
            SURFACE_SESSION,
            "trace-1745",
            &armed_settings(),
        )
        .await;

        // AC3: Passthrough, never Handled — Handled would tell the LLM an action
        // had been taken.
        let enrichment = match action {
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => e,
            other => panic!("expected an enriched Passthrough, got {other:?}"),
        };

        // AC2 (a)-(d): the event, the repo/branch/PR context, the reason, the
        // proposed adoption — and the refusal to act on it.
        assert!(enrichment.contains("CI failure on senara-solutions/mika#1745"));
        assert!(enrichment.contains("feat/1745/gate-surface"));
        assert!(enrichment.contains("No mika task matches this PR"));
        assert!(enrichment.contains("Proposed adoption"));
        assert!(enrichment.contains("Do NOT adopt on your own initiative"));

        assert_eq!(
            audit_count(&db, "pr:senara-solutions/mika#1745@abc123").await,
            1,
            "the audit row is the measurement — it must land"
        );

        let sent = recorder.messages();
        assert_eq!(sent.len(), 1, "exactly one operator notification");
        assert!(
            sent[0].contains("Unadopted work"),
            "the notification names what it is: {}",
            sent[0]
        );
        assert!(
            sent[0].contains("To adopt:"),
            "a signal that does not name its lifting gesture is one you learn to \
             ignore: {}",
            sent[0]
        );
    }

    /// T2 — **negative control (AC4)**, and it is the one that makes T1 mean
    /// something: without it, "the handler decides" is indistinguishable from
    /// "the handler surfaces everything". `claude-pilot-py` routes to mika-dev
    /// (the gateway's `INTERNAL_REPOS`) and the loop may not dispatch into it
    /// (the 2026-08-29 operator decision), so proposing adoption there would
    /// propose an action the mika#2046 gate refuses structurally.
    #[tokio::test]
    async fn mika1745_t2_an_undispatchable_repo_stays_silent() {
        let db = surface_db().await;
        let recorder = Arc::new(RecordingSender::default());
        let sender: Arc<dyn MessageSender> = recorder.clone();

        let action = surface_for_adoption(
            &db,
            &surface_event(NOT_DISPATCHABLE),
            &pr_info("abc123"),
            "https://github.com/senara-solutions/claude-pilot-py/pull/1745",
            Some(&sender),
            SURFACE_SESSION,
            "trace-1745",
            &armed_settings(),
        )
        .await;

        assert!(
            matches!(action, VerdictAction::Passthrough { enrichment: None }),
            "an undispatchable repo keeps the pre-mika#1745 behaviour byte for byte"
        );
        assert_eq!(
            audit_count(&db, "pr:senara-solutions/claude-pilot-py#1745@abc123").await,
            0
        );
        assert!(
            recorder.messages().is_empty(),
            "no notification for a repository out of the loop's reach"
        );
    }

    /// T2-bis — `wizzard`, the other repo in `INTERNAL_REPOS` and not in
    /// `DISPATCHABLE_REPOS`. Named separately because the two are named
    /// separately in the operator decision, and because a predicate that read
    /// `INTERNAL_REPOS` would pass T2 by accident on a one-repo fixture.
    #[tokio::test]
    async fn mika1745_t2bis_wizzard_stays_silent_too() {
        let db = surface_db().await;

        let action = surface_for_adoption(
            &db,
            &surface_event("senara-solutions/wizzard"),
            &pr_info("abc123"),
            "https://github.com/senara-solutions/wizzard/pull/1745",
            None,
            SURFACE_SESSION,
            "trace-1745",
            &armed_settings(),
        )
        .await;

        assert!(matches!(
            action,
            VerdictAction::Passthrough { enrichment: None }
        ));
    }

    /// T2-ter — an unreadable `head_sha` takes the event out of the population.
    /// Without a per-push key the burst of up to 8 workflow events could not be
    /// collapsed, and a per-PR key would suppress every later push for ever — so
    /// the fail-safe direction is silence, not an unkeyed surface.
    #[tokio::test]
    async fn mika1745_t2ter_an_unreadable_head_sha_stays_silent() {
        for sha in ["", "   ", "\t\n"] {
            let db = surface_db().await;
            let recorder = Arc::new(RecordingSender::default());
            let sender: Arc<dyn MessageSender> = recorder.clone();

            let action = surface_for_adoption(
                &db,
                &surface_event(DISPATCHABLE),
                &pr_info(sha),
                "https://github.com/senara-solutions/mika/pull/1745",
                Some(&sender),
                SURFACE_SESSION,
                "trace-1745",
                &armed_settings(),
            )
            .await;

            assert!(
                matches!(action, VerdictAction::Passthrough { enrichment: None }),
                "{sha:?} is not a usable dedup key — the surface must be withheld"
            );
            assert!(recorder.messages().is_empty());
        }
    }

    /// T4 — a second event on the same `head_sha`: one notification, one row.
    /// One push fans out to up to 8 workflows (mika#1869), so this is a
    /// viability condition rather than a refinement.
    #[tokio::test]
    async fn mika1745_t4_the_same_head_sha_surfaces_once() {
        let db = surface_db().await;
        let recorder = Arc::new(RecordingSender::default());
        let sender: Arc<dyn MessageSender> = recorder.clone();

        for _ in 0..8 {
            let action = surface_for_adoption(
                &db,
                &surface_event(DISPATCHABLE),
                &pr_info("abc123"),
                "https://github.com/senara-solutions/mika/pull/1745",
                Some(&sender),
                SURFACE_SESSION,
                "trace-1745",
                &armed_settings(),
            )
            .await;
            // The LLM still gets the fact on every pass — what is withheld is the
            // second notification and the second row.
            assert!(matches!(
                action,
                VerdictAction::Passthrough {
                    enrichment: Some(_)
                }
            ));
        }

        assert_eq!(
            audit_count(&db, "pr:senara-solutions/mika#1745@abc123").await,
            1,
            "a burst of 8 workflow events is one fact"
        );
        assert_eq!(
            recorder.messages().len(),
            1,
            "eight notifications for one fact is how a signal gets muzzled"
        );
    }

    /// T5 — a different `head_sha` is a new fact. The negative control of T4:
    /// without it, "the dedup is per-push" is indistinguishable from "the dedup
    /// is per-PR, for ever".
    #[tokio::test]
    async fn mika1745_t5_a_new_head_sha_surfaces_again() {
        let db = surface_db().await;
        let recorder = Arc::new(RecordingSender::default());
        let sender: Arc<dyn MessageSender> = recorder.clone();

        for sha in ["abc123", "def456"] {
            surface_for_adoption(
                &db,
                &surface_event(DISPATCHABLE),
                &pr_info(sha),
                "https://github.com/senara-solutions/mika/pull/1745",
                Some(&sender),
                SURFACE_SESSION,
                "trace-1745",
                &armed_settings(),
            )
            .await;
        }

        assert_eq!(
            audit_count(&db, "pr:senara-solutions/mika#1745@abc123").await,
            1
        );
        assert_eq!(
            audit_count(&db, "pr:senara-solutions/mika#1745@def456").await,
            1
        );
        assert_eq!(
            recorder.messages().len(),
            2,
            "a second push is a second fact"
        );
    }

    /// T6 — disarmed: the **row is present** and the notification is absent.
    /// This is D4 (`detection is unconditional, only the disposition is gated`)
    /// made assertable, and it is what makes the disarmed mode an observation
    /// mode rather than a blindfold.
    #[tokio::test]
    async fn mika1745_t6_disarmed_still_measures() {
        let db = surface_db().await;
        let recorder = Arc::new(RecordingSender::default());
        let sender: Arc<dyn MessageSender> = recorder.clone();

        let action = surface_for_adoption(
            &db,
            &surface_event(DISPATCHABLE),
            &pr_info("abc123"),
            "https://github.com/senara-solutions/mika/pull/1745",
            Some(&sender),
            SURFACE_SESSION,
            "trace-1745",
            &disarmed_settings(),
        )
        .await;

        assert!(
            matches!(
                action,
                VerdictAction::Passthrough {
                    enrichment: Some(_)
                }
            ),
            "the enrichment is not the disposition — it stays"
        );
        assert_eq!(
            audit_count(&db, "pr:senara-solutions/mika#1745@abc123").await,
            1,
            "detection is unconditional: the measurement must survive the disarm"
        );
        assert!(
            recorder.messages().is_empty(),
            "the notification is what the kill-switch withholds"
        );
    }

    /// T7 — fail-open on an unreadable dedup read. Pure, because the async body's
    /// `Err` arm is not reachable from an in-memory database.
    #[test]
    fn mika1745_t7_an_unreadable_dedup_surfaces_anyway() {
        assert!(
            !is_already_surfaced(Err(())),
            "a database that will not answer must not suppress the surface: a lost \
             surface reopens the defect, a duplicate costs one line"
        );
        // And the two readable tiers, which are what make the line above a
        // decision rather than a constant.
        assert!(!is_already_surfaced(Ok(0)));
        assert!(is_already_surfaced(Ok(1)));
        assert!(is_already_surfaced(Ok(8)));
    }

    /// T8 (AC3, structural) — nothing is adopted: no task row is created, no
    /// status is written, and the action is never `Handled` or `Dispatched`.
    #[tokio::test]
    async fn mika1745_t8_nothing_is_adopted() {
        let db = surface_db().await;

        let action = surface_for_adoption(
            &db,
            &surface_event(DISPATCHABLE),
            &pr_info("abc123"),
            "https://github.com/senara-solutions/mika/pull/1745",
            None,
            SURFACE_SESSION,
            "trace-1745",
            &armed_settings(),
        )
        .await;

        assert!(
            matches!(
                action,
                VerdictAction::Passthrough {
                    enrichment: Some(_)
                }
            ),
            "Handled would tell the LLM an action was taken; Dispatched would be one"
        );
        assert_eq!(
            db.list_active_tasks().await.expect("tasks readable").len(),
            0,
            "AC3: the gate is a decision point, not an action"
        );
    }

    /// T3 — **the negative control the behavioural tests structurally cannot
    /// give.** `surface_for_adoption` is only ever reached from the `None` arm of
    /// the task correlation, so no call to it can demonstrate that a PR *with* a
    /// task is untouched. What can is the source: exactly one production call
    /// site, and it sits inside that arm.
    ///
    /// Without this, "the handler decides on the absence of a task" would be
    /// indistinguishable from "the handler surfaces every CI failure" — and the
    /// regression would make no assertion above go red.
    #[test]
    fn mika1745_t3_the_only_call_site_is_the_no_task_arm() {
        const SRC: &str = include_str!("ci_failure_handler.rs");
        let production = match SRC.find("\n#[cfg(test)]") {
            Some(i) => &SRC[..i],
            None => SRC,
        };

        // **The definition is not a call site**, and the term that excludes it
        // is measured rather than anticipated: the first cut of this scan
        // counted 2 and named `async fn surface_for_adoption(` as a caller. A
        // definition is the one occurrence whose preceding token is `fn`.
        let calls: Vec<usize> = production
            .match_indices("surface_for_adoption(\n")
            .map(|(i, _)| i)
            .filter(|i| !production[..*i].ends_with("fn "))
            .collect();
        assert_eq!(
            calls.len(),
            1,
            "exactly one production call site expected, found {}",
            calls.len()
        );

        let arm_start = production
            .find("let task = match find_active_task(db, &pr_url, &event.branch).await {")
            .expect("the task-correlation match must exist — renamed?");
        let arm_end = production
            .find("// 5. Task must be in_progress")
            .expect("step 5 must follow the correlation — reordered?");
        assert!(
            calls[0] > arm_start && calls[0] < arm_end,
            "the call must sit inside the task-correlation match, so a PR that HAS \
             a task cannot reach it"
        );
    }

    /// The skip motives are a **wire format**: they are published as operator
    /// surfaces, and an operator greps and groups on them, so two spellings of
    /// one motive would split a population without saying so.
    ///
    /// Frozen here rather than left to the emission sites, for the same reason
    /// `worktree_reaper` freezes its own: a rename is a **break to date** in
    /// `CLAUDE.md`, never a silent test update.
    #[test]
    fn mika1745_the_skip_motives_are_a_wire_format() {
        assert_eq!(
            ALL_SURFACE_SKIP_MOTIVES,
            &["repo_not_dispatchable", "unreadable_head_sha"],
            "these values are published in docs/architecture/mika-dev-work-assignment.md \
             § 7 and in the root CLAUDE.md — changing one is a documented break"
        );
        assert_eq!(
            SURFACE_SKIPPED_EVENT, "surface_for_adoption_skipped",
            "the skip event name is greppable and published"
        );

        // The skip event must stay DISTINCT from the SOLE WRITER signal name:
        // folding them would make the two populations impossible to count apart.
        assert_ne!(SURFACE_SKIPPED_EVENT, SURFACE_FOR_ADOPTION_TOOL);
    }

    /// A third skip motive declared and not listed above would leave the frozen
    /// list silently incomplete — the property a production `ALL_*` list gets
    /// from being read by production code, and which `#[cfg(test)]` gives up.
    ///
    /// Scanning the **declarations** rather than the emission sites is what makes
    /// this cheap and exact: every motive is a `const SURFACE_SKIP_*: &str`, one
    /// per line, so there is no expression form to parse.
    #[test]
    fn mika1745_every_declared_skip_motive_is_in_the_wire_format_list() {
        const SRC: &str = include_str!("ci_failure_handler.rs");
        let prefix = format!("const SURFACE{}", "_SKIP_");

        let declared: Vec<&str> = SRC
            .lines()
            .filter_map(|l| l.trim().strip_prefix(prefix.as_str()))
            .filter_map(|rest| rest.split_once("&str = \""))
            .filter_map(|(_, value)| value.split_once('"'))
            .map(|(value, _)| value)
            .collect();

        // Anti-vacuity: a scan that finds no declaration reads exactly like a
        // scan over a complete list (mika#2205).
        assert_eq!(
            declared.len(),
            ALL_SURFACE_SKIP_MOTIVES.len(),
            "declared skip motives {declared:?} do not match the frozen list \
             {ALL_SURFACE_SKIP_MOTIVES:?} — a new motive must join the list, or \
             its value is published nowhere"
        );
        for motive in &declared {
            assert!(
                ALL_SURFACE_SKIP_MOTIVES.contains(motive),
                "{motive:?} is declared but absent from the frozen wire-format list"
            );
        }
    }

    /// The enrichment and the notification must not trip the completion-claim
    /// guard, like every other message this file composes.
    #[test]
    fn mika1745_surface_messages_avoid_completion_claim_words() {
        let event = surface_event(DISPATCHABLE);
        for text in [
            format_surface_enrichment(&event, 1745),
            format_surface_notification(
                &event,
                1745,
                "https://github.com/senara-solutions/mika/pull/1745",
            ),
        ] {
            assert!(
                !COMPLETION_CLAIM_RE.is_match(&text),
                "surface message contains a completion-claim trigger word: {text}"
            );
        }
    }

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
