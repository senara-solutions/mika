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
use crate::task_state::tasks::{
    SUPERSEDED_BY_NEW_DISPATCH, TRACKING_ROW_SUPERSEDED_TOOL, strip_groom_phase_suffix,
};
use tracing::{info, warn};

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
