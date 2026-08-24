//! Integration tests for the phantom NULL-PID sweep (mika#1712).
//!
//! Verifies AC3 (watchdog tick sweep), AC5 (startup sweep), and AC7 (per-row
//! audit-event telemetry) end-to-end via the [`TaskEngine`] surface. Tests
//! inject phantom-shape rows directly via DB (mirroring the leak class the
//! plan targets) and assert the load-bearing audit-events count delta:
//! baseline `count == 0` → post-sweep `count == 1` (or `== 2` for the startup
//! two-row case).
//!
//! Injection-verification recipe (MANDATORY, per plan Phase 5):
//! comment out the `log_audit_event` write inside
//! `TaskEngine::sweep_null_pid_phantoms` (or
//! `sweep_null_pid_phantoms_at_startup`) and re-run these tests — the
//! `count == 1`/`count == 2` assertions must fail. Restore, re-run, assertions
//! pass. The tests thereby prove the audit-events write path is load-bearing.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::tools::default_tools;

const AGENT_ID: &str = "mika";

/// No-op message sender for engine construction — the sweep never sends
/// messages, so the sender is invoked only if a stray dispatch fires.
struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Build an in-memory `AsyncDatabase` scoped to [`AGENT_ID`].
fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Build a minimal `TaskDispatcher` for engine construction. `cli_mode=true`
/// short-circuits the callback dispatch path so the tick loop stays focused
/// on the sweep-relevant scans.
fn test_dispatcher(db: AsyncDatabase) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    Arc::new(TaskDispatcher {
        db,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from("/tmp"),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: true,
        settings,
        pr_reviews_posted: None,
    })
}

/// Insert a phantom-shape tracking row: `trigger_type='manual'`,
/// `action_type='none'`, `process_id IS NULL`. Sets `status` and backdates
/// `updated_at` by `age_secs` seconds (0 = now). Returns the row id.
async fn seed_phantom_row(db: &AsyncDatabase, label: &str, status: &str, age_secs: i64) -> String {
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
            created_by_session: Some("eval-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("create phantom row");

    // Transition to the target status (create_task lands as pending).
    db.update_task_status(&id, status)
        .await
        .expect("set status");

    // Backdate updated_at via the test-only helper. See
    // `Database::backdate_task_updated_at`.
    db.backdate_task_updated_at(&id, age_secs)
        .await
        .expect("backdate updated_at");

    id
}

/// Test 1 — AC3 + AC7: aged phantom row is swept by one tick past the 60-tick
/// interval; audit_events(phantom_aged_out) count goes 0 → 1; the audit row's
/// target_key contains the injected task id (row-shape assertion).
///
/// Load-bearing: if the sweep call is removed from `tick()`, or if
/// `sweep_null_pid_phantoms` early-returns, or if the `log_audit_event` write
/// is commented out, the `count == 1` assertion fails. Covers all three
/// failure modes.
#[tokio::test]
async fn phantom_ages_out_after_grace() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // 3700s past the default 3600s grace → within the AC3 sweep window.
        let task_id = seed_phantom_row(&db, "track mika#1583", "in_progress", 3700).await;

        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            0,
            "baseline: no phantom_aged_out audit events before the sweep"
        );

        // Drive 60 ticks so the periodic DB-scan block fires (which calls
        // sweep_null_pid_phantoms). Loop-tight: this is exactly what the plan
        // asks — "if the call from tick() is commented out OR
        // sweep_null_pid_phantoms is early-returned, this assertion fails".
        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "aged phantom must transition to failed via the sweep"
        );

        let count = db
            .count_audit_events_by_tool_name("phantom_aged_out")
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "AC7 load-bearing: exactly one phantom_aged_out audit event per swept row"
        );

        // Row-shape assertion (T-1/T-2, 2026-08-21): full audit-row shape.
        // target_key references the injected task id; before_value carries the
        // pre-sweep status; after_value == "failed"; reasoning starts with the
        // AC3 source discriminator prefix.
        let expected_key = format!("task:{task_id}");
        let rows = db
            .get_audit_event_rows_by_tool_name("phantom_aged_out")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "exactly one audit row");
        let (target_key, before, after, reasoning) = &rows[0];
        assert_eq!(
            target_key, &expected_key,
            "target_key must be {expected_key}"
        );
        assert_eq!(
            before.as_deref(),
            Some("in_progress"),
            "T-1: before_value must be the pre-sweep status"
        );
        assert_eq!(
            after.as_deref(),
            Some("failed"),
            "T-1: after_value must be 'failed'"
        );
        let reasoning_str = reasoning
            .as_deref()
            .expect("T-2: reasoning must be populated");
        assert!(
            reasoning_str.starts_with("phantom_aged_out:"),
            "T-2: reasoning must carry the AC3 source discriminator prefix — got: {reasoning_str}"
        );
    })
    .await
    .expect("phantom_ages_out_after_grace timed out");
}

/// Test 2 — AC3 age guard: a fresh manual/none/in_progress/NULL-PID row is
/// NOT swept before its grace window elapses. Ticks 61 times (past the 60-tick
/// scan) and asserts `status` stays `in_progress` AND count stays 0.
#[tokio::test]
async fn fresh_manual_none_row_not_swept() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // age_secs=0 → updated_at=now, well within the 3600s default grace.
        let task_id = seed_phantom_row(&db, "fresh track", "in_progress", 0).await;

        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            0
        );

        for _ in 0..61 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "in_progress",
            "fresh row must NOT be swept — age guard defends legitimate long-running tracking"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            0,
            "no audit event should fire when no row is swept"
        );
    })
    .await
    .expect("fresh_manual_none_row_not_swept timed out");
}

/// Test 3 — AC5: `startup_recovery` transitions ALL pre-existing phantoms to
/// `failed` regardless of freshness (age=0 predicate). audit_events count
/// goes 0 → 2.
///
/// Load-bearing: if the startup step 2b is removed, or if
/// `sweep_null_pid_phantoms_at_startup` early-returns, or if the audit
/// `log_audit_event` write is commented out, the `count == 2` assertion fails.
#[tokio::test]
async fn startup_sweep_clears_preexisting_phantoms() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();

        // Seed BEFORE constructing the engine so both are visible at startup.
        // One fresh, one aged — startup sweep must pick up BOTH (age=0).
        let id_a = seed_phantom_row(&db, "preexisting_a", "in_progress", 0).await;
        let id_b = seed_phantom_row(&db, "preexisting_b", "blocked", 7200).await;

        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            0
        );

        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);
        engine.startup_recovery().await.expect("startup_recovery");

        let task_a = db.get_task(&id_a).await.unwrap().unwrap();
        let task_b = db.get_task(&id_b).await.unwrap().unwrap();
        assert_eq!(task_a.status, "failed", "startup sweep transitioned row A");
        assert_eq!(task_b.status, "failed", "startup sweep transitioned row B");

        let count = db
            .count_audit_events_by_tool_name("phantom_aged_out")
            .await
            .unwrap();
        assert_eq!(
            count, 2,
            "AC7 load-bearing: two audit events for two swept rows"
        );

        // T-2 (2026-08-21): AC5 startup-sweep reasoning discriminator must
        // land on both rows. Distinguishes AC5 from AC3 in offline forensics.
        let rows = db
            .get_audit_event_rows_by_tool_name("phantom_aged_out")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        for (_, before, after, reasoning) in &rows {
            assert!(
                matches!(before.as_deref(), Some("in_progress" | "blocked")),
                "T-1: before_value must be the pre-sweep status"
            );
            assert_eq!(
                after.as_deref(),
                Some("failed"),
                "T-1: after_value must be 'failed'"
            );
            let reasoning_str = reasoning
                .as_deref()
                .expect("T-2: reasoning must be populated");
            assert!(
                reasoning_str.starts_with("startup_sweep:"),
                "T-2: reasoning must carry the AC5 source discriminator prefix — got: {reasoning_str}"
            );
        }
    })
    .await
    .expect("startup_sweep_clears_preexisting_phantoms timed out");
}
