//! Supersede-on-new-dispatch cleanup for phantom tracking rows (mika#1934 AC2).
//!
//! When a dispatch-write path creates a fresh tracking row for a `reference_url`
//! that already has an active phantom row in `blocked`/`in_progress`, the older
//! row is a stale escalation artefact — the ticket it tracked was resolved
//! out-of-band, and the escalation surface never terminal-marked it. This module
//! cancels those older rows (`result='superseded_by_new_dispatch'`) BEFORE the
//! new row is inserted, killing the multi-dispatch-collision class (mika#1574 × 4
//! becomes 1 active + 3 cleanly `cancelled`).
//!
//! **Fail-open by construction.** Superseding is a courtesy cleanup, never a
//! precondition for dispatch. Every DB error is logged and swallowed so a
//! transient failure cannot block the actual dispatch — the mika#1712 sweep is
//! the backstop for anything this misses.
//!
//! Shared by both dispatch-write paths (mika#1934 AC2): the engine-side
//! `server::ready_label_handler` and the LLM-facing `tools::create_task`
//! grooming branch.

use crate::async_db::AsyncDatabase;
use crate::task_engine::process_kill::{
    CANCEL_REASON_SUPERSEDED, kill_process_gracefully, pre_write_cancel_reason,
};
use crate::task_state::tasks::{
    SUPERSEDED_BY_NEW_DISPATCH, TRACKING_ROW_SUPERSEDED_TOOL, strip_groom_phase_suffix,
};
use tracing::{info, warn};

/// `audit_events.tool_name` for a live pilot disposed of by a fresh dispatch
/// (mika#2263 défaut (a)). Distinct from [`TRACKING_ROW_SUPERSEDED_TOOL`],
/// which counts phantom ROWS: a superseded row and a killed PROCESS are two
/// different events, and collapsing them would make "how often did the loop
/// leave a zombie behind" uncountable — the one number this fix exists to
/// drive to zero.
pub const SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL: &str = "superseded_dispatch_process_killed";

/// Cancel every active phantom tracking row that a fresh dispatch for
/// `reference_url` (+ `label`) supersedes, emitting one
/// `tracking_row_superseded` audit event per cancelled row. Returns the count
/// of rows actually superseded.
///
/// Coverage (mika#1934 AC2.2 / AC2.3):
/// - the exact `reference_url` (canonicalized — a `?phase=groom` suffix is
///   stripped so a groom dispatch supersedes the base ready-label row too),
/// - the `<base>?phase=groom` variant, and
/// - the NULL-URL label-match fallback (`find_active_task_by_label`) for
///   LLM retry-cycle rows created without a `reference_url`.
///
/// Fail-open: on any DB error the affected step is skipped with a `warn!` and
/// the dispatch proceeds. Never returns `Err`.
pub async fn supersede_prior_tracking_rows(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: Option<&str>,
    reference_url: &str,
    label: &str,
) -> usize {
    let base_url = strip_groom_phase_suffix(reference_url);

    // Collect candidate (task_id, reference_url) pairs, deduped by id. URL-variant
    // branch first, then the NULL-URL label fallback.
    //
    // mika#2335 — computed ONCE, before the kill, and reused for the row
    // cancellation below. Before this, the two halves ran two independent
    // lookups that could disagree; now the set that loses its pilot and the set
    // that loses its row are the same set, by construction.
    let mut candidates: Vec<(String, Option<String>)> = Vec::new();

    match db
        .find_active_tracking_rows_by_reference_url_and_variants(base_url)
        .await
    {
        Ok(rows) => {
            for t in rows {
                candidates.push((t.id, t.reference_url));
            }
        }
        Err(e) => {
            warn!(
                event = "tracking_supersede_lookup_failed",
                reference_url = %reference_url,
                error = %e,
                "supersede: URL-variant lookup failed (fail-open, dispatch proceeds)"
            );
        }
    }

    // NULL-URL label fallback (AC2.3): the exact label carried by the new
    // dispatch may match a prior retry-cycle row created without a reference_url.
    match db.find_active_task_by_label(label).await {
        Ok(Some(t)) => {
            if !candidates.iter().any(|(id, _)| *id == t.id) {
                candidates.push((t.id, t.reference_url));
            }
        }
        Ok(None) => {}
        Err(e) => {
            warn!(
                event = "tracking_supersede_label_lookup_failed",
                label = %label,
                error = %e,
                "supersede: label-match lookup failed (fail-open, dispatch proceeds)"
            );
        }
    }

    // mika#2263 défaut (a), population corrigée par mika#2335 — dispose of the
    // PROCESSES before the ROWS.
    //
    // Ordered before the row bookkeeping deliberately: the fresh dispatch is
    // about to take the same worktree, and the whole point is that no prior
    // pilot is still writing to it when that happens. Killing afterwards would
    // still leave a window where two writers share an arbre (classe
    // #2248/#2249) — which is exactly the 28 min 54 s overlap measured on
    // 2026-09-15.
    dispose_superseded_dispatch_processes(db, session_id, trace_id, base_url, &candidates).await;

    let mut superseded = 0usize;
    for (task_id, row_ref_url) in candidates {
        match db.cancel_task_superseded(&task_id).await {
            Ok(true) => {
                superseded += 1;
                let reasoning =
                    format!("superseded by fresh dispatch for {reference_url} (label: {label})");
                if let Err(e) = db
                    .log_audit_event(
                        session_id,
                        TRACKING_ROW_SUPERSEDED_TOOL,
                        &format!("task:{task_id}"),
                        row_ref_url.as_deref(),
                        Some(SUPERSEDED_BY_NEW_DISPATCH),
                        Some(&reasoning),
                        trace_id,
                    )
                    .await
                {
                    warn!(
                        event = "tracking_supersede_audit_failed",
                        task_id = %task_id,
                        error = %e,
                        "supersede: failed to write audit event (non-fatal)"
                    );
                }
                info!(
                    event = "tracking_row_superseded",
                    task_id = %task_id,
                    reference_url = %reference_url,
                    "supersede: cancelled prior phantom tracking row"
                );
            }
            // Row transitioned to terminal between lookup and cancel, or was not
            // in the phantom shape — no-op, not an error.
            Ok(false) => {}
            Err(e) => {
                warn!(
                    event = "tracking_supersede_cancel_failed",
                    task_id = %task_id,
                    error = %e,
                    "supersede: guarded cancel failed (fail-open, dispatch proceeds)"
                );
            }
        }
    }

    superseded
}

/// Kill the pilot process of every live dispatch hanging under the tracking
/// rows a fresh dispatch supersedes, and mark the child row terminal
/// (mika#2263 défaut (a), population corrigée par mika#2335).
///
/// **The defect mika#2263 tried to close.** Superseding cancelled the ROW and
/// ignored the PROCESS. On 2026-09-09 `#2252` and `#2212` each ended with a
/// `cancelled` row and a bwrap pilot still alive on its worktree — 69 min and
/// 45 min of a burned dispatch slot, plus a second writer on an arbre a fresh
/// dispatch was about to claim.
///
/// **Why it did not close it.** mika#2263 resolved the population with a second
/// SQL resolver, keyed on `reference_url` **and** `process_id IS NOT NULL` —
/// a conjunction that is empty on the topology production writes. A dispatch is
/// two rows: the **parent** tracking row carries the issue URL and never a
/// pgid; the **callback child** carries the pgid and never a URL. So the kill
/// could not find anyone, in this incident or in any other. Measured on
/// 2026-09-15: the log of the surviving pilot `590a06c0` contains zero
/// occurrences of `SIGTERM`, `CANCELLED_BY`, `superseded` or `Killed` — the
/// supersession did not miss it narrowly, it never saw it.
///
/// **The link was already there.** `find_dispatch_children_with_pid` (mika#2156)
/// does exactly the parent → child-with-pgid traversal, and its own doc says
/// it: *"the recall row already points back via `parent_task_id` — the missing
/// link is read here, not added."* One join predicate, reused; the filtering
/// decision stays here, at its caller. Writing a third resolver is what
/// produced this defect once already.
///
/// Per child: pre-write the cancel-reason file (so `dispatch-lib`'s TERM trap
/// names [`CANCEL_REASON_SUPERSEDED`] instead of the generic
/// `CANCELLED_BY_SIGNAL`), SIGTERM → grace → SIGKILL the process **group** —
/// possible because the spawn makes the child a group leader
/// (`.process_group(0)`, `skills/executor.rs`) — then `cancelled` the child row
/// and clear its `process_id` so no later reaper re-signals a PID that may
/// since have been reused.
///
/// **Two filters bound the population to dispatches genuinely in flight**, and
/// both matter now that the kill can actually land:
///
/// - a child whose status is terminal (`delivered`, `cancelled`, `failed`, …)
///   has no pilot left to kill and its `process_id` is a stale pgid;
/// - a child with no readable `process_start_time` is **not signalled**. Without
///   it `kill_process_gracefully` falls back to a bare `/proc/<pid>` existence
///   check, which cannot tell our pilot from a recycled PID — and signalling a
///   process group by mistake is unbounded damage. The cost is named rather
///   than hidden: such a child survives its supersession. Inertia, never a
///   blind kill. Counted and logged under mika#2156's vocabulary
///   (`unusable_child_count`).
///
/// **Fail-open, like everything on this path.** Superseding is a courtesy
/// cleanup, never a precondition for dispatch: every DB error is logged and
/// swallowed. A kill that does not land leaves the pre-#2263 behaviour, which
/// the mtime-worktree reaper (mika#2249/#2277) and the PID watchdog (#959)
/// still backstop.
///
/// `parents` is the candidate set [`supersede_prior_tracking_rows`] already
/// computed — passed in rather than re-derived, so the rows that lose their
/// pilot and the rows that lose their status are the same rows.
///
/// Returns the count of processes actually signalled.
pub async fn dispose_superseded_dispatch_processes(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: Option<&str>,
    base_url: &str,
    parents: &[(String, Option<String>)],
) -> usize {
    let mut killed = 0usize;
    let mut unusable_child_count = 0usize;

    for (parent_id, parent_ref_url) in parents {
        let children = match db.find_dispatch_children_with_pid(parent_id).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    event = "superseded_dispatch_lookup_failed",
                    task_id = %parent_id,
                    reference_url = %base_url,
                    error = %e,
                    "supersede: dispatch-child lookup failed (fail-open, dispatch proceeds)"
                );
                continue;
            }
        };

        for child in children {
            if is_terminal_status(&child.status) {
                continue;
            }

            // Fail-safe anti-PID-reuse: no start time, no signal.
            let Some(start_time) = child.process_start_time else {
                unusable_child_count += 1;
                warn!(
                    event = "superseded_dispatch_unusable_child",
                    task_id = %parent_id,
                    child_task_id = %child.id,
                    pid = child.process_id,
                    "supersede: dispatch child carries a pgid but no readable \
                     process_start_time — not signalled, since a recycled PID \
                     would be indistinguishable from the pilot"
                );
                continue;
            };

            let pid = child.process_id;
            pre_write_cancel_reason(pid, CANCEL_REASON_SUPERSEDED);
            let dead = kill_process_gracefully(pid, Some(start_time)).await;
            killed += 1;

            // Terminal-mark the CHILD through the ordinary cancel path — the
            // supersede-specific `cancel_task_superseded` refuses anything
            // carrying a `process_id`, which is precisely the shape being
            // disposed of here. The parent is cancelled by the caller, through
            // that guarded path, once this returns.
            if let Err(e) = db.cancel_task(&child.id).await {
                warn!(
                    event = "superseded_dispatch_cancel_failed",
                    task_id = %child.id,
                    error = %e,
                    "supersede: failed to cancel live dispatch row after kill (non-fatal)"
                );
            }
            if let Err(e) = db.clear_task_process_id(&child.id).await {
                warn!(
                    event = "superseded_dispatch_clear_pid_failed",
                    task_id = %child.id,
                    error = %e,
                    "supersede: failed to clear process_id after kill (non-fatal)"
                );
            }

            let reasoning = format!(
                "live pilot pid={pid} killed={dead} on child of {parent_id}, \
                 superseded by fresh dispatch for {base_url}"
            );
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL,
                    &format!("task:{}", child.id),
                    parent_ref_url.as_deref(),
                    Some(CANCEL_REASON_SUPERSEDED),
                    Some(&reasoning),
                    trace_id,
                )
                .await
            {
                warn!(
                    event = "superseded_dispatch_audit_failed",
                    task_id = %child.id,
                    error = %e,
                    "supersede: failed to write process-kill audit event (non-fatal)"
                );
            }

            info!(
                event = "superseded_dispatch_process_killed",
                task_id = %parent_id,
                child_task_id = %child.id,
                pid,
                dead,
                reference_url = %base_url,
                "supersede: disposed of the pilot a fresh dispatch replaces"
            );
        }
    }

    if unusable_child_count > 0 {
        info!(
            event = "superseded_dispatch_complete",
            killed,
            unusable_child_count,
            reference_url = %base_url,
            "supersede: some dispatch children survived their supersession for \
             want of a readable process_start_time"
        );
    }

    killed
}

/// Statuses on which a dispatch child no longer has a pilot to kill.
///
/// Deliberately a positive list of terminal states rather than `!= pending &&
/// != in_progress`: an unknown status must read as *not terminal*, so a state
/// added later is still disposed of rather than silently spared.
///
/// Written with the `task_status` constants rather than bare literals, for the
/// reason their own module gives: a typo in a status string compiles and then
/// spares a live pilot in silence.
fn is_terminal_status(status: &str) -> bool {
    use crate::task_engine::types::task_status;
    matches!(
        status,
        task_status::DELIVERED
            | task_status::COMPLETED
            | task_status::CANCELLED
            | task_status::FAILED
            | task_status::EXPIRED
    )
}
