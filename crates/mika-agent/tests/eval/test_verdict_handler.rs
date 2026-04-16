//! Integration tests for the structural PR review verdict handler (#524).
//!
//! These tests exercise `try_handle_pr_review_verdict` with a real AsyncDatabase
//! to verify work item lookups, metadata updates, and audit event logging.
//! The `gh` subprocess calls (run_gh_checks, run_gh_merge) are NOT called because
//! these tests use scenarios that short-circuit before reaching the merge path
//! (no matching work item, wrong status, block verdict, etc.).

use anyhow::Result;
use serde_json::json;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::server::verdict_handler::{VerdictAction, try_handle_pr_review_verdict};
use mika_agent::task_engine::types::{action_type, trigger_type};

/// Default agent ID used by `AsyncDatabase::new()`.
const AGENT_ID: &str = "mika";
const SESSION_ID: &str = "verdict-test-session";

/// Helper to create an in-memory async database for tests.
async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory db");
    // create_session is a sync method on Database — must be called before wrapping
    db.create_session(SESSION_ID, AGENT_ID, "github")
        .expect("create session");
    AsyncDatabase::new(db)
}

/// Helper to create a work item with PR URL in metadata.
async fn create_work_item_with_pr_url(db: &AsyncDatabase, pr_url: &str) -> String {
    let metadata = json!({
        "claude_pilot": {
            "pr_url": pr_url,
            "branch": "feat/test"
        }
    });

    let task_id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: format!("Test work item for {pr_url}"),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION_ID.to_string()),
            created_trace_id: None,
            reference_url: Some(pr_url.to_string()),
            source: Some("github_issue".to_string()),
            metadata: Some(serde_json::to_string(&metadata).unwrap()),
            r#type: None,
        })
        .await
        .expect("create task");

    // Transition to in_progress
    db.update_manual_task_status(&task_id, "in_progress")
        .await
        .expect("update to in_progress");

    task_id
}

/// Build a gateway-formatted PR review event text.
fn pr_review_text(state: &str, repo: &str, number: u64, reviewer: &str, body: &str) -> String {
    format!(
        "[GitHub] PR review ({state}) on {repo}#{number} (feat: test feature) by @{reviewer}\n\
         https://github.com/{repo}/pull/{number}#pullrequestreview-12345\n\
         \n\
         {body}"
    )
}

// -------------------------------------------------------------------------
// Test: block verdict -> Passthrough (R7)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ci_passes_through() -> Result<()> {
    let db = test_db().await;
    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: block[ci]\n\nCI checks are failing.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(enrichment.is_none(), "block verdict should not enrich");
        }
        VerdictAction::Handled { .. } => {
            panic!("block verdict should not be handled structurally");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: hold verdict -> Passthrough
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_hold_review_passes_through() -> Result<()> {
    let db = test_db().await;
    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: hold[review]\n\nNeeds another look.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(enrichment.is_none(), "hold verdict should not enrich");
        }
        VerdictAction::Handled { .. } => {
            panic!("hold verdict should not be handled structurally");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: missing verdict -> Passthrough with enrichment (R4)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_missing_passes_through_with_enrichment() -> Result<()> {
    let db = test_db().await;
    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "Looks good, approved!",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            let e = enrichment.expect("missing verdict should enrich");
            assert!(e.contains("verdict_missing=true"));
        }
        VerdictAction::Handled { .. } => {
            panic!("missing verdict should not be handled structurally");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: non-PR-review event -> Passthrough (no verdict handling)
// -------------------------------------------------------------------------

#[tokio::test]
async fn non_pr_review_event_passes_through() -> Result<()> {
    let db = test_db().await;
    let text = "[GitHub] Issue assigned: senara-solutions/mika#100 — fix: bug\n\
                https://github.com/senara-solutions/mika/issues/100\n\n\
                Assigned to: @mika-dev";

    let action =
        try_handle_pr_review_verdict(text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(
                enrichment.is_none(),
                "non-review event should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("non-review event should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict but no matching work item -> Passthrough
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_no_work_item_passes_through() -> Result<()> {
    let db = test_db().await;
    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        999,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(
                enrichment.is_none(),
                "pass with no work item should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with no work item should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict with work item in completed status -> Passthrough (R5)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_completed_work_item_passes_through() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task_id = create_work_item_with_pr_url(&db, pr_url).await;

    // Transition to completed (terminal)
    db.update_manual_task_status(&task_id, "completed")
        .await
        .expect("update to completed");

    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(
                enrichment.is_none(),
                "pass with completed work item should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with completed work item should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict with work item in pending status -> Passthrough (R5)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_pending_work_item_passes_through() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/50";

    let metadata = json!({
        "claude_pilot": {
            "pr_url": pr_url,
        }
    });

    let task_id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Pending work item".to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION_ID.to_string()),
            created_trace_id: None,
            reference_url: Some(pr_url.to_string()),
            source: Some("github_issue".to_string()),
            metadata: Some(serde_json::to_string(&metadata).unwrap()),
            r#type: None,
        })
        .await
        .expect("create task");

    // Leave in pending status (don't transition to in_progress)
    let _ = task_id;

    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        50,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(
                enrichment.is_none(),
                "pass with pending work item should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with pending work item should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict but no github token -> Passthrough with enrichment
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_no_github_token_passes_through() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let _task_id = create_work_item_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );

    let action = try_handle_pr_review_verdict(
        &text, &db, None, // no github token
        None, SESSION_ID, "trace-1",
    )
    .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            let e = enrichment.expect("no-token should enrich");
            assert!(e.contains("no GitHub token"));
        }
        VerdictAction::Handled { .. } => {
            panic!("no-token case should pass through, not handle");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: non-approved review state -> Passthrough
// -------------------------------------------------------------------------

#[tokio::test]
async fn non_approved_review_passes_through() -> Result<()> {
    let db = test_db().await;
    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: pass\n\nLooks good.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            assert!(
                enrichment.is_none(),
                "non-approved review should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("non-approved review should not be handled");
        }
    }
    Ok(())
}
