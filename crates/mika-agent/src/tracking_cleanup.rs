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

    // mika#2263 défaut (a) — dispose of the PROCESS before the ROWS.
    //
    // Ordered first deliberately: the fresh dispatch is about to take the same
    // worktree, and the whole point is that no prior pilot is still writing to
    // it when that happens. Killing after the row bookkeeping would still leave
    // a window where two writers share an arbre (classe #2248/#2249).
    dispose_superseded_dispatch_processes(db, session_id, trace_id, base_url).await;

    // Collect candidate (task_id, reference_url) pairs, deduped by id. URL-variant
    // branch first, then the NULL-URL label fallback.
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

/// Kill the pilot process of every LIVE dispatch that a fresh dispatch for
/// `base_url` supersedes, and mark its row terminal (mika#2263 défaut (a)).
///
/// **The defect this closes.** Superseding cancelled the ROW and ignored the
/// PROCESS. On 2026-09-09 `#2252` and `#2212` each ended with a `cancelled`
/// row and a bwrap pilot still alive on its worktree — 69 min and 45 min of a
/// burned dispatch slot, plus a second writer on an arbre a fresh dispatch was
/// about to claim.
///
/// Per row: pre-write the cancel-reason file (so `dispatch-lib`'s TERM trap
/// names [`CANCEL_REASON_SUPERSEDED`] instead of the generic
/// `CANCELLED_BY_SIGNAL`), SIGTERM → grace → SIGKILL the process **group**,
/// then `cancelled` the row and clear its `process_id` so no later reaper
/// re-signals a PID that may since have been reused.
///
/// **Fail-open, like everything on this path.** Superseding is a courtesy
/// cleanup, never a precondition for dispatch: every DB error is logged and
/// swallowed. A kill that does not land leaves the pre-#2263 behaviour, which
/// the mtime-worktree reaper (mika#2249) and the PID watchdog still backstop.
///
/// Returns the count of processes actually signalled.
pub async fn dispose_superseded_dispatch_processes(
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: Option<&str>,
    base_url: &str,
) -> usize {
    let rows = match db
        .find_live_dispatch_rows_by_reference_url_and_variants(base_url)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                event = "superseded_dispatch_lookup_failed",
                reference_url = %base_url,
                error = %e,
                "supersede: live-dispatch lookup failed (fail-open, dispatch proceeds)"
            );
            return 0;
        }
    };

    let mut killed = 0usize;
    for task in rows {
        let Some(pid) = task.process_id else { continue };

        // PID-reuse guard input (#855). Absent metadata degrades to the basic
        // /proc existence check, exactly as on the operator cancel path.
        let start_time: Option<u64> = task
            .metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("process_start_time")?.as_str()?.parse().ok());

        pre_write_cancel_reason(pid, CANCEL_REASON_SUPERSEDED);
        let dead = kill_process_gracefully(pid, start_time).await;
        killed += 1;

        // Terminal-mark the row through the ordinary cancel path — the
        // supersede-specific `cancel_task_superseded` refuses anything carrying
        // a `process_id`, which is precisely the shape being disposed of here.
        match db.cancel_task(&task.id).await {
            Ok(_) => {}
            Err(e) => warn!(
                event = "superseded_dispatch_cancel_failed",
                task_id = %task.id,
                error = %e,
                "supersede: failed to cancel live dispatch row after kill (non-fatal)"
            ),
        }
        if let Err(e) = db.clear_task_process_id(&task.id).await {
            warn!(
                event = "superseded_dispatch_clear_pid_failed",
                task_id = %task.id,
                error = %e,
                "supersede: failed to clear process_id after kill (non-fatal)"
            );
        }

        let reasoning = format!(
            "live pilot pid={pid} killed={dead} superseded by fresh dispatch for {base_url}"
        );
        if let Err(e) = db
            .log_audit_event(
                session_id,
                SUPERSEDED_DISPATCH_PROCESS_KILLED_TOOL,
                &format!("task:{}", task.id),
                task.reference_url.as_deref(),
                Some(CANCEL_REASON_SUPERSEDED),
                Some(&reasoning),
                trace_id,
            )
            .await
        {
            warn!(
                event = "superseded_dispatch_audit_failed",
                task_id = %task.id,
                error = %e,
                "supersede: failed to write process-kill audit event (non-fatal)"
            );
        }

        info!(
            event = "superseded_dispatch_process_killed",
            task_id = %task.id,
            pid,
            dead,
            reference_url = %base_url,
            "supersede: disposed of the pilot a fresh dispatch replaces"
        );
    }

    killed
}
