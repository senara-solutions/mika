//! Integration tests for supersede-on-new-dispatch tracking-row cleanup
//! (mika#1934 AC2 / AC3).
//!
//! Exercises the exact pre-create path `ready_label_handler` and the
//! `create_task` grooming branch run before inserting a fresh tracking row:
//! `mika_agent::tracking_cleanup::supersede_prior_tracking_rows`. Each test
//! seeds a phantom-shape tracking row (`trigger_type='manual'`,
//! `action_type='none'`, `process_id IS NULL`, `status IN ('blocked',
//! 'in_progress')`), invokes the supersede path, and asserts the row transitions
//! to `cancelled` with `result='superseded_by_new_dispatch'` plus exactly one
//! `tracking_row_superseded` audit event per superseded row.
//!
//! Load-bearing (injection check): comment out the `cancel_task_superseded` call
//! inside `supersede_prior_tracking_rows` and these assertions fail; restore and
//! they pass — proving the transition is the load-bearing effect.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH;
use mika_agent::tracking_cleanup::supersede_prior_tracking_rows;

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";
const SUPERSEDE_TOOL: &str = "tracking_row_superseded";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Seed a phantom-shape tracking row with the given `status`, `reference_url`,
/// and `label`. Returns the row id.
async fn seed_tracking_row(
    db: &AsyncDatabase,
    label: &str,
    status: &str,
    reference_url: Option<&str>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: None,
            reference_url: reference_url.map(|s| s.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create tracking row");
    db.update_task_status(&id, status)
        .await
        .expect("set status");
    id
}

async fn supersede_count(db: &AsyncDatabase) -> i64 {
    db.count_audit_events_by_tool_name(SUPERSEDE_TOOL)
        .await
        .unwrap()
}

/// AC7.1 — blocked → superseded (primary path). Seeded blocked row for an issue
/// URL is cancelled with the canonical reason; a fresh row can then be created
/// for the same URL; exactly one audit event is written.
#[tokio::test]
async fn blocked_row_superseded_by_new_dispatch() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/9999";
    let old_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#9999",
        "blocked",
        Some(url),
    )
    .await;

    assert_eq!(
        supersede_count(&db).await,
        0,
        "baseline: no supersede events"
    );

    let n = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#9999",
    )
    .await;
    assert_eq!(n, 1, "exactly one row superseded");

    let old = db.get_task(&old_id).await.unwrap().unwrap();
    assert_eq!(old.status, "cancelled", "seeded row must be cancelled");
    assert_eq!(
        old.result.as_deref(),
        Some(SUPERSEDED_BY_NEW_DISPATCH),
        "result must carry the canonical supersede reason"
    );

    assert_eq!(
        supersede_count(&db).await,
        1,
        "AC3: exactly one tracking_row_superseded audit event per superseded row"
    );

    // Fresh row now inserts cleanly for the same URL (index slot freed).
    let new_id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "ready-label: senara-solutions/mika#9999".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: None,
            reference_url: Some(url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("fresh row inserts after supersede");
    let new = db.get_task(&new_id).await.unwrap().unwrap();
    assert_eq!(new.status, "pending", "fresh row lands as pending");
    assert_ne!(new_id, old_id, "fresh row is a distinct row");
}

/// AC7.2 — in_progress → superseded (load-bearing race path). The mika#1574 × 4
/// retry-cycle pattern relies on in_progress supersede working.
#[tokio::test]
async fn in_progress_row_superseded_by_new_dispatch() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/1574";
    let old_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#1574",
        "in_progress",
        Some(url),
    )
    .await;

    let n = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#1574",
    )
    .await;
    assert_eq!(n, 1);

    let old = db.get_task(&old_id).await.unwrap().unwrap();
    assert_eq!(old.status, "cancelled");
    assert_eq!(old.result.as_deref(), Some(SUPERSEDED_BY_NEW_DISPATCH));
    assert_eq!(supersede_count(&db).await, 1);
}

/// AC2.2 — a fresh dispatch supersedes BOTH the base ready-label row and the
/// `?phase=groom` variant for the same underlying issue.
#[tokio::test]
async fn url_variant_both_superseded() {
    let db = test_db();
    let base = "https://github.com/senara-solutions/mika/issues/1867";
    let groom = "https://github.com/senara-solutions/mika/issues/1867?phase=groom";
    let base_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#1867",
        "blocked",
        Some(base),
    )
    .await;
    let groom_id = seed_tracking_row(&db, "groom mika#1867", "blocked", Some(groom)).await;

    // Dispatch arrives on the base URL; both rows must be superseded.
    let n = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        base,
        "ready-label: senara-solutions/mika#1867",
    )
    .await;
    assert_eq!(n, 2, "both the base and groom-variant rows are superseded");

    for id in [&base_id, &groom_id] {
        let t = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(t.status, "cancelled", "row {id} cancelled");
        assert_eq!(t.result.as_deref(), Some(SUPERSEDED_BY_NEW_DISPATCH));
    }
    assert_eq!(
        supersede_count(&db).await,
        2,
        "one audit event per superseded row"
    );
}

/// AC2.2 (reverse) — a groom dispatch (`?phase=groom` URL) canonicalizes to the
/// base URL and supersedes the base ready-label row too.
#[tokio::test]
async fn groom_dispatch_supersedes_base_row() {
    let db = test_db();
    let base = "https://github.com/senara-solutions/mika/issues/1712";
    let groom = "https://github.com/senara-solutions/mika/issues/1712?phase=groom";
    let base_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#1712",
        "blocked",
        Some(base),
    )
    .await;

    let n =
        supersede_prior_tracking_rows(&db, SESSION, Some(TRACE), groom, "groom mika#1712").await;
    assert_eq!(
        n, 1,
        "the base row is superseded by the groom-variant dispatch"
    );
    let base_row = db.get_task(&base_id).await.unwrap().unwrap();
    assert_eq!(base_row.status, "cancelled");
}

/// AC2.3 — NULL-URL label-match fallback. A retry-cycle row created without a
/// reference_url is caught by the exact-label fallback.
#[tokio::test]
async fn null_url_label_match_superseded() {
    let db = test_db();
    let label = "groom mika#1574 (auto-groom, poison loop)";
    let old_id = seed_tracking_row(&db, label, "blocked", None).await;

    // The new dispatch carries the SAME label but a URL; the URL-variant lookup
    // finds nothing, and the label fallback catches the NULL-URL row.
    let n = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        "https://github.com/senara-solutions/mika/issues/1574",
        label,
    )
    .await;
    assert_eq!(n, 1, "NULL-URL row superseded via label fallback");
    let old = db.get_task(&old_id).await.unwrap().unwrap();
    assert_eq!(old.status, "cancelled");
    assert_eq!(old.result.as_deref(), Some(SUPERSEDED_BY_NEW_DISPATCH));
}

/// Idempotency — re-invoking supersede after the row is already cancelled does
/// not double-cancel and writes no second audit event. A `pending` row is never
/// superseded (only the phantom blocked/in_progress shape is).
#[tokio::test]
async fn supersede_is_idempotent_and_skips_pending() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/2000";
    let old_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#2000",
        "blocked",
        Some(url),
    )
    .await;

    let first = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2000",
    )
    .await;
    assert_eq!(first, 1);
    let second = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        url,
        "ready-label: senara-solutions/mika#2000",
    )
    .await;
    assert_eq!(second, 0, "already-cancelled row is not re-superseded");
    assert_eq!(supersede_count(&db).await, 1, "no duplicate audit event");
    let _ = old_id;

    // A pending row for a different URL is never superseded.
    let pend_url = "https://github.com/senara-solutions/mika/issues/2001";
    let pend_id = seed_tracking_row(
        &db,
        "ready-label: senara-solutions/mika#2001",
        "pending",
        Some(pend_url),
    )
    .await;
    let n = supersede_prior_tracking_rows(
        &db,
        SESSION,
        Some(TRACE),
        pend_url,
        "ready-label: senara-solutions/mika#2001",
    )
    .await;
    assert_eq!(
        n, 0,
        "pending rows are not phantom-shaped — never superseded"
    );
    let pend = db.get_task(&pend_id).await.unwrap().unwrap();
    assert_eq!(pend.status, "pending", "pending row untouched");
}
