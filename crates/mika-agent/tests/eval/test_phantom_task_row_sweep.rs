//! Integration tests for the phantom NULL-PID sweep (mika#1712).
//!
//! Verifies AC3 (watchdog tick sweep), AC5 (startup sweep), and AC7 (per-row
//! audit-event telemetry) end-to-end via the [`TaskEngine`] surface. Tests
//! inject phantom-shape rows directly via DB (mirroring the leak class the
//! plan targets) and assert the load-bearing audit-events count delta:
//! baseline `count == 0` → post-sweep `count == 1` (or `== 2` for the startup
//! two-row case).
//!
//! Extended for mika#2156 with the liveness-guard cases: age alone cannot
//! tell an orphaned tracking row from one whose dispatch is still running,
//! because the tracking row's `updated_at` is never bumped while the work
//! proceeds. The guard tests below inject a recall child carrying a real
//! `(pid, process_start_time)` pair and assert the row survives — and the
//! symmetric cases (no child, dead child, child without a start time) assert
//! the sweeper is not disarmed.
//!
//! Fixture labels carry accents on purpose, and the reason is the production
//! data rather than a written rule: real `ready-label:` and `groom` rows are
//! titled from French issue titles (see the mika#2151 and mika#2140 rows
//! quoted in the ticket), so an ASCII-only battery would not exercise the
//! population this code actually runs against. The accented cases are on the
//! two sparing tests — the ones the injection check turns red — not only on
//! decorative ones.
//!
//! Honest limit of that: nothing here reads a label back, so the accent is
//! carried through the fixtures but is not itself load-bearing for the guard,
//! which discriminates on `(pid, start_time)`. An encoding regression in the
//! label round-trip would not be caught by this battery.
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
    test_dispatcher_with_grace(db, None)
}

/// Same, with the phantom-sweep grace window pinned. Set through the
/// dispatcher's own `Settings` rather than `MIKA_PHANTOM_SWEEP_AGE_SECONDS`:
/// the env var is process-global, and these tests share a binary with others
/// that run concurrently.
fn test_dispatcher_with_grace(db: AsyncDatabase, grace_secs: Option<u64>) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let mut settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    if grace_secs.is_some() {
        settings.phantom_sweep_age_seconds = grace_secs;
    }
    Arc::new(TaskDispatcher {
        db,
        tier: mika_common::home::AgentTier::Default,
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

/// Insert a recall (`long_running:*`) child row under `parent_id`, carrying
/// `process_id` and — unless `start_time` is `None` — the
/// `metadata.process_start_time` string the executor writes. This is the exact
/// shape measured in production: the tracking row holds no PID of its own, the
/// real process lives on this separate child, and `parent_task_id` already
/// links the two (mika#2156 measure M1).
async fn seed_recall_child(
    db: &AsyncDatabase,
    parent_id: &str,
    label: &str,
    pid: i64,
    start_time: Option<u64>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.to_string()),
            depth: 1,
            label: label.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
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
        .expect("create recall child");

    db.set_task_process_id(&id, Some(pid))
        .await
        .expect("set child process_id");

    if let Some(st) = start_time {
        // The executor stores this as a JSON *string* — mirror it exactly, or
        // the test would validate a shape production never writes.
        db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
            .await
            .expect("set process_start_time");
    }

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

        // Past the default grace (14400s since mika#2156) → within the AC3
        // sweep window.
        let task_id = seed_phantom_row(&db, "suivi mika#1583 — dépôt", "in_progress", 15000).await;

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

        // age_secs=0 → updated_at=now, well within the default grace
        // (14400s since mika#2156).
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

// ---------------------------------------------------------------------------
// mika#2156 — the liveness guard
//
// Age measures time since the tracking row was last *written*, not time since
// the work last showed a sign of life: the row's `updated_at` is frozen a
// second after creation while its dispatch runs for hours. These tests pin the
// discriminator that replaces the clock — is the recall child's process still
// alive? — and its symmetric cases, so the sweeper is not disarmed.
//
// Injection-verification: delete the `live_dispatch_child` guard block from
// `sweep_null_pid_phantoms` and `phantom_sweep_spares_row_with_live_dispatch_child`
// plus `startup_sweep_spares_row_with_live_dispatch_child` must both go red,
// while the three "still reaps" tests stay green.
// ---------------------------------------------------------------------------

/// The test process's own `(pid, start_time)` — a pair guaranteed alive for
/// the duration of the assertion, which is what makes these tests
/// deterministic rather than dependent on spawning and racing a child.
///
/// Linux-only, like the guard itself: `read_process_start_time` is `None` by
/// construction off Linux and `is_same_process_alive` always returns `false`
/// there, so this would panic and the sparing tests would assert something the
/// platform cannot deliver. Callers open with [`skip_off_linux`], following
/// `process_liveness`'s own convention.
fn own_live_process() -> (i64, u64) {
    let pid = std::process::id();
    let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(pid)
        .expect("read own process start time");
    (i64::from(pid), start_time)
}

/// Returns true when the caller should return early: the liveness guard is a
/// Linux-only mechanism (`/proc/<pid>/stat` field 22), so on other platforms
/// skipping is the honest outcome rather than a weakened assertion. Mirrors
/// the `if !cfg!(target_os = "linux") { return; }` convention in
/// `process_liveness`'s own unit tests.
fn skip_off_linux() -> bool {
    !cfg!(target_os = "linux")
}

/// Test 4 — AC2: the measured case. Tracking row `action_type='none'`,
/// `process_id IS NULL`, `in_progress`, `updated_at` two hours old, WITH a
/// recall child whose process is alive → the tracking row survives.
///
/// This is mika#2156's founding incident in miniature: the row was declared
/// `phantom_aged_out` while its pilot went on writing for another 2h08.
#[tokio::test]
async fn phantom_sweep_spares_row_with_live_dispatch_child() {
    tokio::time::timeout(Duration::from_secs(5), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        // Pin the pre-fix hour so this test exercises the guard, not the
        // raised threshold: at 7200s the row IS a sweep candidate, and only
        // liveness can save it.
        let dispatcher = test_dispatcher_with_grace(db.clone(), Some(3600));
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // Two hours old — the window the ticket was measured in. Accented
        // label: our real tracking rows carry French text, so the fixture
        // must too.
        let task_id = seed_phantom_row(
            &db,
            "ready-label: senara-solutions/mika#2151 — dépêche différée",
            "in_progress",
            7200,
        )
        .await;

        let (pid, start_time) = own_live_process();
        let child_id = seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot",
            pid,
            Some(start_time),
        )
        .await;

        // Control in the same pass. Without it this test asserts only that a
        // row did NOT change, which also holds if the sweep never ran or the
        // row was never a candidate — and candidacy here rests on a pinned
        // grace that the default just moved past (3600 -> 14400). The orphan
        // proves the pass really swept at a grace this row crosses.
        let orphan_id = seed_phantom_row(
            &db,
            "fiche orpheline — même passe, sans rappel",
            "in_progress",
            7200,
        )
        .await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "in_progress",
            "AC2: a tracking row backed by a live dispatch child must survive the sweep, \
             however old its updated_at — the row is not the work"
        );
        assert!(
            task.result.is_none(),
            "AC2: no failure reason must be written on a spared row — got {:?}",
            task.result
        );

        let orphan = db.get_task(&orphan_id).await.unwrap().unwrap();
        assert_eq!(
            orphan.status, "failed",
            "control: the sweep must have actually run over this pass at a grace \
             both rows cross — otherwise the assertions above are vacuous"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            1,
            "exactly one transition: the orphan, not the live row"
        );

        // AC4, asserted rather than eyeballed: the withheld transition leaves a
        // durable row naming the recall child and the retained process_id.
        let spare_rows = db
            .get_audit_event_rows_by_tool_name("phantom_sweep_spared")
            .await
            .unwrap();
        assert_eq!(spare_rows.len(), 1, "AC4: one audit row per spared row");
        let (target_key, before, after, reasoning) = &spare_rows[0];
        assert_eq!(target_key, &format!("task:{task_id}"));
        assert_eq!(
            (before.as_deref(), after.as_deref()),
            (Some("in_progress"), Some("in_progress")),
            "AC4: a spare moves nothing — both sides are the unchanged status"
        );
        let reasoning = reasoning.as_deref().expect("AC4: reasoning populated");
        assert!(
            reasoning.contains(&child_id),
            "AC4: the recall child's id must be readable after the fact — got {reasoning}"
        );
        assert!(
            reasoning.contains(&pid.to_string()),
            "AC4: the retained process_id must be readable after the fact — got {reasoning}"
        );

        // The child is untouched — the guard reads it, never writes it.
        let child = db.get_task(&child_id).await.unwrap().unwrap();
        assert_eq!(child.process_id, Some(pid));
    })
    .await
    .expect("phantom_sweep_spares_row_with_live_dispatch_child timed out");
}

/// Test 5 — AC3: same tracking row, no recall child at all → still swept.
/// The genuine orphans, which are the sweeper's reason to exist, keep being
/// reaped.
#[tokio::test]
async fn phantom_sweep_still_reaps_row_without_live_child() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2140 — fiche orpheline, sans rappel",
            "in_progress",
            15000,
        )
        .await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "AC3: with no dispatch child, the sweeper must still reap — the guard \
             withholds transitions, it does not disable them"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            1,
            "AC3: the swept row still emits its audit event"
        );
    })
    .await
    .expect("phantom_sweep_still_reaps_row_without_live_child timed out");
}

/// Test 6 — AC3, and the control that matters. Same tracking row WITH a recall
/// child, but the child's process is gone → still swept.
///
/// This is the case that separates "there is a child" from "the child lives".
/// Measure M2 of the plan found 177 of 181 historical sweeps had a
/// PID-carrying child, so a guard written on the child's mere existence would
/// have disarmed the sweeper on 98% of its population — and this test is what
/// catches that mistake.
#[tokio::test]
async fn phantom_sweep_still_reaps_row_with_dead_child() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2169 — rappel mort, fiche stérile",
            "in_progress",
            15000,
        )
        .await;
        // PID 999_999_999 is the impossible PID already used as the dead-process
        // control in `process_liveness`'s own unit tests.
        seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot",
            999_999_999,
            Some(12_345),
        )
        .await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "AC3: a child whose process is dead must not spare the row — liveness is \
             the discriminator, not the child's existence"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            1
        );
    })
    .await
    .expect("phantom_sweep_still_reaps_row_with_dead_child timed out");
}

/// Test 7 — D-3: a child carrying a live PID but no `process_start_time` is
/// not enough to spare the row. Without the start time the `(pid, start_time)`
/// pair that identifies a process *instance* is incomplete, so a recycled PID
/// would read as alive. Fail toward the pre-fix behaviour: sweep.
#[tokio::test]
async fn phantom_sweep_reaps_child_without_start_time() {
    tokio::time::timeout(Duration::from_secs(5), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2158 — rappel sans horodatage de démarrage",
            "in_progress",
            15000,
        )
        .await;
        let (pid, _) = own_live_process();
        seed_recall_child(&db, &task_id, "long_running:run_claude_pilot", pid, None).await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "D-3: a live PID with no stored start_time cannot rule out PID reuse — \
             the guard must not spare on it"
        );
    })
    .await
    .expect("phantom_sweep_reaps_child_without_start_time timed out");
}

/// Test 8 — AC1 + AC3 on the startup path, where the guard matters most:
/// `startup_recovery` sweeps at age=0, so there is no grace window to absorb a
/// mistake. A pilot survives an engine restart (it is its own process-group
/// leader, mika#855), so without the guard every restart would mark `failed`
/// the whole set of dispatches actually in flight.
#[tokio::test]
async fn startup_sweep_spares_row_with_live_dispatch_child() {
    tokio::time::timeout(Duration::from_secs(5), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();

        // Seeded before the engine exists, as at a real boot.
        let live_id = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — dispatch encore en vol au redémarrage",
            "in_progress",
            0,
        )
        .await;
        let (pid, start_time) = own_live_process();
        seed_recall_child(
            &db,
            &live_id,
            "long_running:run_claude_pilot",
            pid,
            Some(start_time),
        )
        .await;

        // Control in the same pass: a genuine orphan must still be reaped, or
        // this test would pass on a sweeper that simply stopped running.
        let orphan_id = seed_phantom_row(&db, "fiche orpheline au démarrage", "blocked", 0).await;

        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);
        engine.startup_recovery().await.expect("startup_recovery");

        let live = db.get_task(&live_id).await.unwrap().unwrap();
        assert_eq!(
            live.status, "in_progress",
            "AC1: at age=0 the startup sweep must still consult liveness — a pilot \
             outlives an engine restart"
        );

        let orphan = db.get_task(&orphan_id).await.unwrap().unwrap();
        assert_eq!(
            orphan.status, "failed",
            "AC3: the orphan in the same pass must still be reaped"
        );

        assert_eq!(
            db.count_audit_events_by_tool_name("phantom_aged_out")
                .await
                .unwrap(),
            1,
            "exactly one audit event: the orphan, not the live row"
        );
    })
    .await
    .expect("startup_sweep_spares_row_with_live_dispatch_child timed out");
}

/// Test 9 — the loop, not just the predicate. A row with TWO PID-carrying
/// children where the DEAD one is examined first must still be spared.
///
/// This is the test that goes red if either `continue` in `dispatch_liveness`
/// ever becomes `break` or an early return — the natural simplification of a
/// function described as "returns the first live child". Every other liveness
/// test seeds exactly one child, so without this one that refactor would
/// silently disarm the fix for any parent that accumulated a second dispatch,
/// and the whole suite would stay green.
#[tokio::test]
async fn phantom_sweep_spares_when_only_the_second_child_is_live() {
    tokio::time::timeout(Duration::from_secs(5), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        let dispatcher = test_dispatcher_with_grace(db.clone(), Some(3600));
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — deux rappels, un seul vivant",
            "in_progress",
            7200,
        )
        .await;

        // Seed the dead child first. `find_dispatch_children_with_pid` orders
        // by id (UUIDs), so seeding order does not fix the examination order —
        // the mirror test below covers the other arrangement.
        // Distinct labels: `tasks` carries UNIQUE(agent_id, label), and in
        // production a parent accumulates children of different shapes
        // (`:deferred`, `_groom`, a retry) rather than duplicates.
        //
        // One child per way of NOT being live, all ordered before the live
        // one, so every skip path in the loop is exercised: an unusable pid,
        // a child with no start_time, and a dead process. Any of them
        // short-circuiting the scan loses the live child behind them.
        //
        // The ids are forced because `find_dispatch_children_with_pid` orders
        // by `id` and `create_task` mints UUIDv4 — without this the live child
        // would sort first roughly a quarter of the time and the test would
        // only catch a short-circuit by luck.
        let unusable = seed_recall_child(
            &db,
            &task_id,
            "long_running: pid inutilisable",
            0,
            Some(12_345),
        )
        .await;
        let no_stamp = seed_recall_child(
            &db,
            &task_id,
            "long_running: sans horodatage",
            424_242,
            None,
        )
        .await;
        let dead = seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot:deferred",
            999_999_999,
            Some(12_345),
        )
        .await;
        let (pid, start_time) = own_live_process();
        let live = seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot",
            pid,
            Some(start_time),
        )
        .await;
        for (id, forced) in [
            (unusable, "aaa-1-pid-inutilisable"),
            (no_stamp, "aaa-2-sans-horodatage"),
            (dead, "aaa-3-processus-mort"),
            (live, "zzz-4-vivant"),
        ] {
            db.set_task_id_for_test(&id, forced)
                .await
                .expect("force child id");
        }

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "in_progress",
            "a dead child must not stop the scan — the guard's contract is \
             'all children dead', not 'the first child is dead'"
        );
    })
    .await
    .expect("phantom_sweep_spares_when_only_the_second_child_is_live timed out");
}

/// Test 10 — PID reuse, composed through the guard rather than asserted on the
/// primitive. `process_liveness` unit-tests `is_same_process_alive` with a
/// wrong start time; this pins that the sweep path actually consults it. A
/// child holding a LIVE pid with the WRONG start_time must not spare: the pair
/// identifies a process instance, and half of it matching is not a match.
#[tokio::test]
async fn phantom_sweep_still_reaps_row_with_recycled_pid() {
    tokio::time::timeout(Duration::from_secs(5), async {
        if skip_off_linux() {
            return;
        }
        let db = test_db();
        let dispatcher = test_dispatcher_with_grace(db.clone(), Some(3600));
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — PID recyclé, instance différente",
            "in_progress",
            7200,
        )
        .await;
        let (pid, start_time) = own_live_process();
        seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot",
            pid,
            // Live PID, wrong instance.
            Some(start_time + 1),
        )
        .await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "a recycled PID must not spare — the start_time is what makes the \
             pair identify an instance"
        );
    })
    .await
    .expect("phantom_sweep_still_reaps_row_with_recycled_pid timed out");
}

/// Test 11 — `process_id = 0` must not spare.
///
/// Load-bearing rather than defensive: `kill(0, 0)` targets the *caller's own*
/// process group, so without the `p > 0` filter a zero-PID child would read as
/// alive and spare its row forever. Deleting that filter (or coercing with
/// `as u32`) passes every other test in this file.
#[tokio::test]
async fn phantom_sweep_still_reaps_row_with_zero_pid_child() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        let dispatcher = test_dispatcher_with_grace(db.clone(), Some(3600));
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let task_id = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — rappel à PID zéro",
            "in_progress",
            7200,
        )
        .await;
        seed_recall_child(
            &db,
            &task_id,
            "long_running:run_claude_pilot",
            0,
            Some(12_345),
        )
        .await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "failed",
            "pid 0 must never read as alive — kill(0, 0) would hit our own \
             process group"
        );
    })
    .await
    .expect("phantom_sweep_still_reaps_row_with_zero_pid_child timed out");
}

/// Test 12 — the raised threshold itself, in the engine path.
///
/// Measured while writing this test, and worth knowing before reading it: a
/// tracking row with **no children at all** never reaches the phantom sweep's
/// grace, because the childless-parent reaper takes it at
/// `CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS` (1800s) with
/// `stuck_in_progress_no_callback_child`. So the raised default only ever
/// governs rows that HAVE a child — which is also the shape of a dispatch
/// waiting for a slot, whose deferred wrapper carries no `process_id`.
///
/// Both rows below therefore carry a PID-less child: that isolates the
/// phantom-sweep threshold from the other reaper, and models the queued state.
/// Reverting `DEFAULT_PHANTOM_SWEEP_AGE_SECONDS` to 3600 turns this red.
#[tokio::test]
async fn row_between_old_and_new_grace_is_not_swept() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let db = test_db();
        // Default settings on purpose — this test is about the default.
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let young = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — deux heures, en attente de créneau",
            "in_progress",
            7200,
        )
        .await;
        seed_recall_child(&db, &young, "long_running:...:deferred (jeune)", 0, None).await;

        // Control: past the new default, so the sweeper is provably running.
        let old = seed_phantom_row(
            &db,
            "ready-label: mika#2156 — au-delà du nouveau seuil",
            "in_progress",
            15000,
        )
        .await;
        seed_recall_child(&db, &old, "long_running:...:deferred (vieux)", 0, None).await;

        for _ in 0..60 {
            engine.tick().await;
        }

        let y = db.get_task(&young).await.unwrap().unwrap();
        assert_eq!(
            y.status, "in_progress",
            "a 2h-old row is within the 14400s default and must survive — got \
             result {:?}",
            y.result
        );
        let o = db.get_task(&old).await.unwrap().unwrap();
        assert_eq!(o.status, "failed", "control: the sweeper ran this pass");
        assert_eq!(
            o.result.as_deref(),
            Some("phantom_aged_out"),
            "control must be reaped by THIS sweeper, not by a sibling reaper"
        );
    })
    .await
    .expect("row_between_old_and_new_grace_is_not_swept timed out");
}
