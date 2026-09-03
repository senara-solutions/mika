//! Integration tests for complete-on-upstream-close tracking-row cleanup
//! (mika#1934 AC4 / AC5 / AC7.3).
//!
//! Fires synthetic `issues.closed` and `pull_request.closed` webhook payloads
//! against `server::upstream_close_handler::try_handle_upstream_close` and
//! asserts the matching phantom tracking row transitions to its terminal state
//! with the correct `result`, plus exactly one `tracking_row_upstream_closed`
//! audit event per transitioned row. Idempotency is covered by re-firing the
//! same event.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::server::upstream_close_handler::try_handle_upstream_close;
use mika_agent::task_state::tasks::{
    ISSUE_CLOSED_UPSTREAM, UPSTREAM_PR_CLOSED_UNMERGED, UPSTREAM_PR_MERGED,
};

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";
const UPSTREAM_TOOL: &str = "tracking_row_upstream_closed";

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

async fn seed_blocked_row(db: &AsyncDatabase, label: &str, reference_url: &str) -> String {
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
            reference_url: Some(reference_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create blocked row");
    db.update_task_status(&id, "blocked")
        .await
        .expect("set blocked");
    id
}

async fn upstream_count(db: &AsyncDatabase) -> i64 {
    db.count_audit_events_by_tool_name(UPSTREAM_TOOL)
        .await
        .unwrap()
}

/// AC7.3 — issues.closed → cancelled with `issue_closed_upstream` + one audit.
#[tokio::test]
async fn issues_closed_cancels_tracking_row() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/9999";
    let id = seed_blocked_row(&db, "ready-label: senara-solutions/mika#9999", url).await;

    assert_eq!(upstream_count(&db).await, 0, "baseline");

    let text = format!("[GitHub] Issue closed: senara-solutions/mika#9999 — title\n{url}");
    try_handle_upstream_close(&text, &db, SESSION, TRACE).await;

    let t = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(t.status, "cancelled");
    assert_eq!(t.result.as_deref(), Some(ISSUE_CLOSED_UPSTREAM));
    assert_eq!(
        upstream_count(&db).await,
        1,
        "AC5: one tracking_row_upstream_closed audit event"
    );
}

/// AC4 — pull_request.closed (merged) → completed with `upstream_pr_merged`.
/// The tracking row is keyed on the ISSUE the PR closes, parsed from the body.
#[tokio::test]
async fn pr_closed_merged_completes_tracking_row() {
    let db = test_db();
    let issue_url = "https://github.com/senara-solutions/mika/issues/8888";
    let id = seed_blocked_row(&db, "ready-label: senara-solutions/mika#8888", issue_url).await;

    let text = "[GitHub] PR closed: senara-solutions/mika#500 — fix (branch: fix/8888)\n\
                https://github.com/senara-solutions/mika/pull/500\n\
                Merged: true\n\n\
                This PR Closes #8888."
        .to_string();
    try_handle_upstream_close(&text, &db, SESSION, TRACE).await;

    let t = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(t.status, "completed", "merged PR completes the row");
    assert_eq!(t.result.as_deref(), Some(UPSTREAM_PR_MERGED));
    assert_eq!(upstream_count(&db).await, 1);
}

/// AC4 — pull_request.closed (unmerged) → cancelled with
/// `upstream_pr_closed_unmerged`.
#[tokio::test]
async fn pr_closed_unmerged_cancels_tracking_row() {
    let db = test_db();
    let issue_url = "https://github.com/senara-solutions/mika/issues/7777";
    let id = seed_blocked_row(&db, "ready-label: senara-solutions/mika#7777", issue_url).await;

    let text = "[GitHub] PR closed: senara-solutions/mika#501 — wip (branch: wip/7777)\n\
                https://github.com/senara-solutions/mika/pull/501\n\
                Merged: false\n\n\
                Fixes #7777."
        .to_string();
    try_handle_upstream_close(&text, &db, SESSION, TRACE).await;

    let t = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(t.status, "cancelled");
    assert_eq!(t.result.as_deref(), Some(UPSTREAM_PR_CLOSED_UNMERGED));
    assert_eq!(upstream_count(&db).await, 1);
}

/// AC4.c — idempotency: re-firing the same issues.closed event does not
/// re-transition the (now terminal) row and writes no second audit event.
#[tokio::test]
async fn issues_closed_is_idempotent() {
    let db = test_db();
    let url = "https://github.com/senara-solutions/mika/issues/6666";
    let id = seed_blocked_row(&db, "ready-label: senara-solutions/mika#6666", url).await;

    let text = format!("[GitHub] Issue closed: senara-solutions/mika#6666 — title\n{url}");
    try_handle_upstream_close(&text, &db, SESSION, TRACE).await;
    try_handle_upstream_close(&text, &db, SESSION, TRACE).await;

    let t = db.get_task(&id).await.unwrap().unwrap();
    assert_eq!(t.status, "cancelled");
    assert_eq!(
        upstream_count(&db).await,
        1,
        "AC4.c: re-delivery writes no duplicate audit event"
    );
}

/// AC4.c — no-op safety: an issues.closed event for an issue with no tracking
/// row transitions nothing and writes no audit event.
#[tokio::test]
async fn issues_closed_no_matching_row_is_noop() {
    let db = test_db();
    let text = "[GitHub] Issue closed: senara-solutions/mika#4242 — title\n\
                https://github.com/senara-solutions/mika/issues/4242";
    try_handle_upstream_close(text, &db, SESSION, TRACE).await;
    assert_eq!(
        upstream_count(&db).await,
        0,
        "no matching row → no audit event, no WARN"
    );
}
