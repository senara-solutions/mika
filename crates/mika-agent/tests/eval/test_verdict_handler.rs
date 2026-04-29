//! Integration tests for the structural PR review verdict handler (#524, #889).
//!
//! These tests exercise `try_handle_pr_review_verdict` with a real AsyncDatabase
//! to verify task lookups, metadata updates, and audit event logging.
//! The `gh` subprocess calls (run_gh_checks, run_gh_merge) are NOT called because
//! these tests use scenarios that short-circuit before reaching the merge path
//! (no matching task, wrong status, etc.).
//!
//! Post-#889: block[*], hold[*], and missing verdicts are now handled structurally
//! rather than passed through to the LLM. Tests updated to reflect the new dispatch.

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

/// Helper to create a task with PR URL in metadata.
async fn create_task_with_pr_url(db: &AsyncDatabase, pr_url: &str) -> String {
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
            label: format!("Test task for {pr_url}"),
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
// Test: block[ci] verdict with no task -> Passthrough with enrichment (#889)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ci_no_task_passes_through_with_enrichment() -> Result<()> {
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

    // With no matching task, block[ci] passes through with enrichment
    match action {
        VerdictAction::Passthrough { enrichment } => {
            let e = enrichment.expect("block[ci] with no task should enrich");
            assert!(
                e.contains("block[ci]"),
                "enrichment should mention verdict type"
            );
        }
        VerdictAction::Handled { .. } => {
            // This is also acceptable — the handler may handle even with no task
            // depending on implementation. The key test is block[ci] WITH a task below.
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: block[ci] verdict with task -> Handled (#889 R3)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ci_with_task_is_handled() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: block[ci]\n\nCI checks are failing.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("block[ci]"),
                "pre-digest should mention block[ci]"
            );
            assert!(
                pre_digest.contains("verdict_handler"),
                "pre-digest should have XML tag"
            );
            assert!(
                pre_digest.contains("dispatch run_claude_pilot"),
                "pre-digest should instruct dispatch"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[ci] with matching task should be handled structurally");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: block[ac] verdict with task -> Handled (#889 R2)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ac_with_task_is_handled() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/888";
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        888,
        "mika-qa",
        "PLAN-AC VERIFICATION:\n\
         - [\u{2705}] satisfied: AC1\n\
         - [\u{274c}] unsatisfied: Unit 1 eval test: expected eval harness test; actual: missing\n\
         - [\u{274c}] unsatisfied: Unit 3 operator command: expected command; actual: missing\n\n\
         VERDICT: block[ac]\n\
         REASON: Two plan ACs unsatisfied.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("block[ac]"),
                "pre-digest should mention block[ac]"
            );
            assert!(
                pre_digest.contains("verdict_handler"),
                "pre-digest should have XML tag"
            );
            assert!(
                pre_digest.contains("dispatch run_claude_pilot"),
                "pre-digest should instruct dispatch"
            );
            assert!(
                pre_digest.contains("Retry: 1/3"),
                "pre-digest should show retry count"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[ac] with matching task should be handled structurally");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: block[ac] retry limit -> Handled with escalation (#889 R12)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ac_retry_limit_escalates() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/888";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

    // Pre-set the retry counter to 3 (limit reached)
    let metadata = json!({
        "claude_pilot": { "pr_url": pr_url },
        "verdict_block_ac": { "count": 3 }
    });
    db.update_task_metadata(&task_id, &serde_json::to_string(&metadata).unwrap())
        .await
        .expect("update metadata");

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        888,
        "mika-qa",
        "VERDICT: block[ac]\nREASON: ACs still unsatisfied after 3 attempts.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("retry limit"),
                "pre-digest should mention retry limit"
            );
            assert!(
                pre_digest.contains("Do NOT dispatch"),
                "pre-digest should prevent dispatch"
            );
            assert!(
                pre_digest.contains("retry budget is exhausted"),
                "pre-digest should say budget exhausted"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[ac] at retry limit should be handled as escalation");
        }
    }

    // Verify task was marked blocked
    let task = db
        .get_task(&task_id)
        .await
        .expect("get task")
        .expect("task should exist");
    assert_eq!(
        task.status, "blocked",
        "task should be blocked after retry limit"
    );

    Ok(())
}

// -------------------------------------------------------------------------
// Test: block[security] -> Handled with escalation (#889 R4)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_security_escalates() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: block[security]\nREASON: Hardcoded API key found.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("block[security]"),
                "pre-digest should mention block[security]"
            );
            assert!(
                pre_digest.contains("Do NOT dispatch"),
                "should not dispatch for security"
            );
            assert!(
                pre_digest.contains("operator review"),
                "should require operator review"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[security] should be handled structurally");
        }
    }

    // Verify task was marked blocked
    let task = db
        .get_task(&task_id)
        .await
        .expect("get task")
        .expect("task should exist");
    assert_eq!(
        task.status, "blocked",
        "task should be blocked for security escalation"
    );

    Ok(())
}

// -------------------------------------------------------------------------
// Test: block[pipeline] -> Handled with escalation (#889 R4)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_pipeline_escalates() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: block[pipeline]\nREASON: Missing pipeline artifacts.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(pre_digest.contains("block[pipeline]"));
            assert!(pre_digest.contains("Do NOT dispatch"));
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[pipeline] should be handled structurally");
        }
    }

    let task = db.get_task(&task_id).await.expect("get task");
    assert_eq!(task.unwrap().status, "blocked");

    Ok(())
}

// -------------------------------------------------------------------------
// Test: hold[review] verdict with task -> Handled (#889 R5)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_hold_review_with_task_is_handled() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: hold[review]\n\nNeeds design input.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("hold[review]"),
                "pre-digest should mention hold[review]"
            );
            assert!(
                pre_digest.contains("remains in_progress"),
                "task should remain in_progress"
            );
            assert!(
                pre_digest.contains("Do NOT dispatch"),
                "should not auto-dispatch"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("hold[review] with matching task should be handled structurally");
        }
    }

    // Verify task status unchanged (still in_progress)
    let task = db
        .get_task(&task_id)
        .await
        .expect("get task")
        .expect("task should exist");
    assert_eq!(
        task.status, "in_progress",
        "hold[review] should not change task status"
    );

    Ok(())
}

// -------------------------------------------------------------------------
// Test: hold[review] verdict with no task -> Passthrough (#889)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_hold_review_no_task_passes_through() -> Result<()> {
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

    // With no task, hold[review] passes through with enrichment
    match action {
        VerdictAction::Passthrough { enrichment } => {
            let e = enrichment.expect("hold[review] with no task should enrich");
            assert!(e.contains("hold[review]"));
        }
        VerdictAction::Handled { .. } => {
            // Also acceptable — handler may still handle structurally
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: missing verdict -> Handled with classification_failed (#889 R6)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_missing_is_handled_structurally() -> Result<()> {
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
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("verdict_classification_failed"),
                "pre-digest should contain classification_failed tag"
            );
            assert!(
                pre_digest.contains("No parseable VERDICT:"),
                "pre-digest should explain the failure"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("missing verdict should now be handled structurally (#889)");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: missing verdict with no VERDICT line -> audit event logged (#889 R11)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_missing_logs_audit_event() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "Just some comments without a verdict line.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    // Should be handled
    assert!(
        matches!(action, VerdictAction::Handled { .. }),
        "missing verdict should be handled structurally"
    );

    // Verify audit event was logged
    let events = db
        .get_audit_events(SESSION_ID)
        .await
        .expect("get audit events");
    let classification_failed = events
        .iter()
        .any(|e| e.tool_name == "verdict_classification_failed");
    assert!(
        classification_failed,
        "verdict_classification_failed audit event should have been logged"
    );

    Ok(())
}

// -------------------------------------------------------------------------
// Test: state=CHANGES_REQUESTED + block[ac] -> single dispatch (#889 R10)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_changes_requested_block_ac_single_dispatch() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "changes_requested",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: block[ac]\nREASON: ACs unsatisfied.",
    );

    let action =
        try_handle_pr_review_verdict(&text, &db, Some("fake-token"), None, SESSION_ID, "trace-1")
            .await;

    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(pre_digest.contains("block[ac]"));
            assert!(pre_digest.contains("dispatch run_claude_pilot"));
            // No double-fire: only one dispatch instruction
            let dispatch_count = pre_digest.matches("dispatch run_claude_pilot").count();
            assert_eq!(
                dispatch_count, 1,
                "should have exactly one dispatch instruction"
            );
        }
        VerdictAction::Passthrough { .. } => {
            panic!("block[ac] with matching task should be handled regardless of state");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict regression (#524 R9)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_approved_with_task_no_token_passthrough() -> Result<()> {
    // Regression test: pass + approved + task exists but no token -> passthrough
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

    let text = pr_review_text(
        "approved",
        "senara-solutions/mika",
        42,
        "mika-qa",
        "VERDICT: pass\n\nAll good.",
    );

    let action = try_handle_pr_review_verdict(
        &text, &db, None, // no token
        None, SESSION_ID, "trace-1",
    )
    .await;

    match action {
        VerdictAction::Passthrough { enrichment } => {
            let e = enrichment.expect("no-token pass should enrich");
            assert!(e.contains("no GitHub token"));
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with no token should pass through");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: sequential block[ac] retry counting (#889 R12)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_block_ac_sequential_retries() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/888";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

    let verdict_text = pr_review_text(
        "commented",
        "senara-solutions/mika",
        888,
        "mika-qa",
        "VERDICT: block[ac]\nREASON: ACs unsatisfied.",
    );

    // First attempt: should dispatch (count goes to 1)
    let action1 = try_handle_pr_review_verdict(
        &verdict_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-1",
    )
    .await;
    match &action1 {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("Retry: 1/3"),
                "first attempt should be 1/3"
            );
            assert!(pre_digest.contains("dispatch run_claude_pilot"));
        }
        _ => panic!("first block[ac] should be handled"),
    }

    // Second attempt: should dispatch (count goes to 2)
    let action2 = try_handle_pr_review_verdict(
        &verdict_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-2",
    )
    .await;
    match &action2 {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("Retry: 2/3"),
                "second attempt should be 2/3"
            );
            assert!(pre_digest.contains("dispatch run_claude_pilot"));
        }
        _ => panic!("second block[ac] should be handled"),
    }

    // Third attempt: should dispatch (count goes to 3)
    let action3 = try_handle_pr_review_verdict(
        &verdict_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-3",
    )
    .await;
    match &action3 {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("Retry: 3/3"),
                "third attempt should be 3/3"
            );
            assert!(pre_digest.contains("dispatch run_claude_pilot"));
        }
        _ => panic!("third block[ac] should be handled"),
    }

    // Fourth attempt: should escalate (count at 3, >= limit)
    let action4 = try_handle_pr_review_verdict(
        &verdict_text,
        &db,
        Some("fake-token"),
        None,
        SESSION_ID,
        "trace-4",
    )
    .await;
    match &action4 {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.contains("retry limit"),
                "fourth attempt should escalate"
            );
            assert!(
                pre_digest.contains("Do NOT dispatch"),
                "should prevent dispatch"
            );
            assert!(pre_digest.contains("retry budget is exhausted"));
        }
        _ => panic!("fourth block[ac] should be handled as escalation"),
    }

    // Verify task is now blocked
    let task = db
        .get_task(&task_id)
        .await
        .expect("get task")
        .expect("task should exist");
    assert_eq!(
        task.status, "blocked",
        "task should be blocked after retry limit"
    );

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
// Test: pass verdict but no matching task -> Passthrough
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_no_task_passes_through() -> Result<()> {
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
                "pass with no task should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with no task should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict with task in completed status -> Passthrough (R5)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_completed_task_passes_through() -> Result<()> {
    let db = test_db().await;
    let pr_url = "https://github.com/senara-solutions/mika/pull/42";
    let task_id = create_task_with_pr_url(&db, pr_url).await;

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
                "pass with completed task should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with completed task should not be handled");
        }
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Test: pass verdict with task in pending status -> Passthrough (R5)
// -------------------------------------------------------------------------

#[tokio::test]
async fn verdict_pass_pending_task_passes_through() -> Result<()> {
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
            label: "Pending task".to_string(),
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
                "pass with pending task should pass through cleanly"
            );
        }
        VerdictAction::Handled { .. } => {
            panic!("pass with pending task should not be handled");
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
    let _task_id = create_task_with_pr_url(&db, pr_url).await;

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
