//! Integration tests for the pilot silent-stall reaper (mika#2249, D1).
//!
//! The class under test is the one no reaper could see before: a dispatch whose
//! **process is alive** but whose worktree has received no write for hours.
//! Every pre-existing reaper fires on task state,
//! and the PID watchdog fires on a **dead** process — so `fb355061` sat for
//! 2 h 18 with an empty `result`, no terminal marker, and no callback, until an
//! operator disposed of it by hand.
//!
//! # This file covers the `in_progress` surface — not the production one
//!
//! Read this before adding a case here (mika#2272). Every test below seeds
//! `in_progress` **by an explicit write**, and production never puts a live
//! dispatch's callback row in that status: it is created `pending` and stays
//! `pending` until the callback lands. That synthetic status is precisely how
//! the reaper could ship, pass this whole file, and be inert in production for
//! a day — the fixture manufactured the population the code knew how to read.
//!
//! The file is kept, and kept on `in_progress`, because the reaper's query
//! covers both surfaces and the second one deserves coverage too. But the
//! **production shape** — a `pending` row seeded through `build_callback_task`,
//! with a real live process — lives in
//! `test_reaper_reaps_live_pending_pilot_2272.rs`, and that is where a new case
//! about "does the reaper actually fire" belongs.
//!
//! # Injection-verification recipe (MANDATORY, plan Phase 5 / AC5)
//!
//! The AC5 test is red on `main` for a structural reason rather than a
//! configured one: `reap_silently_stalled_pilots` does not exist there, and
//! neither does the `pilot_silent_stall` audit name, so the `count == 1`
//! assertion cannot pass. To re-verify against *this* branch, comment out the
//! `self.reap_silently_stalled_pilots().await;` call in `TaskEngine::tick` —
//! the count assertion in [`stalled_pilot_is_detected_and_disposed`] and the
//! status assertion in [`disarmed_it_observes_without_disposing`] must both
//! fail. Restore, re-run, both pass. (Done again on mika#2272, against the
//! production-shape tests in the companion file.)
//!
//! # What these tests deliberately do NOT cover
//!
//! The reaper's own scan cost (AC7) is unit-tested in
//! `task_engine::worktree_activity`, against real directory trees. Rebuilding
//! a `mika`-sized checkout here would buy a slower suite and the same answer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::tools::default_tools;

// Fixtures de processus réels (mika#2265, AC2) : extraites de ce fichier vers un
// module partagé, commentaires load-bearing inclus — le fix reaper de mika#2272
// les réutilise pour ses assertions cycle-de-vie-process.
use super::process_fixtures::{kill_pid, spawn_and_reap_child, spawn_live_child};

const AGENT_ID: &str = "mika";

/// Comfortably past the 2700 s default so the fixtures do not encode the
/// threshold twice — `mika_common::config` owns that number and pins it.
const STALE_SECS: u64 = 10_000;

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Build a dispatcher with the reaper's two knobs pinned through `Settings`
/// rather than through `MIKA_PILOT_STALL_REAP_*`: the env is process-global and
/// this binary runs its tests concurrently.
fn test_dispatcher(db: AsyncDatabase, armed: bool) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let mut settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    settings.pilot_stall_reap_enabled = Some(armed);
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

fn set_mtime(path: &Path, secs_ago: u64) {
    let when = SystemTime::now() - Duration::from_secs(secs_ago);
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when))
        .expect("backdate mtime");
}

/// Build a worktree whose newest write is `idle_secs` old, and return its path.
fn seed_worktree(root: &Path, idle_secs: u64) -> PathBuf {
    let worktree = root.join("worktree");
    let src = worktree.join("crates").join("mika-agent").join("src");
    std::fs::create_dir_all(&src).expect("mkdir worktree");
    let file = src.join("engine.rs");
    std::fs::write(&file, b"fn main() {}").expect("write file");
    // Backdate the whole chain — directories carry their own mtime, so leaving
    // any of them at "now" would make the tree look freshly written.
    for p in [
        file.as_path(),
        src.as_path(),
        &worktree.join("crates").join("mika-agent"),
        &worktree.join("crates"),
        worktree.as_path(),
    ] {
        set_mtime(p, idle_secs);
    }
    worktree
}

/// Write the declaration file `dispatch-lib.sh` writes, and return its path.
fn declare_worktree(root: &Path, worktree: &Path) -> PathBuf {
    let file = root.join("declaration.path");
    std::fs::write(&file, format!("{}\n", worktree.display())).expect("write declaration");
    file
}

/// Seed the `in_progress` surface: a `callback` task moved there by an explicit
/// write, carrying `process_id` plus the two metadata fields the executor
/// stamps.
///
/// The `update_task_status` below is **not** what production writes on this row
/// (mika#2272) — see this module's header. `test_reaper_reaps_live_pending_pilot_2272.rs`
/// carries the production-shape fixture.
async fn seed_dispatch(
    db: &AsyncDatabase,
    label: &str,
    pid: i64,
    start_time: Option<u64>,
    declaration_file: Option<&Path>,
) -> String {
    let id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "callback".to_string(),
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
        .expect("create callback task");

    db.update_task_status(&id, "in_progress")
        .await
        .expect("set in_progress");
    db.set_task_process_id(&id, Some(pid))
        .await
        .expect("set process_id");
    if let Some(st) = start_time {
        // The executor stores this as a JSON *string* — mirror it exactly, or
        // the fixture would validate a shape production never writes.
        db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
            .await
            .expect("set process_start_time");
    }
    if let Some(path) = declaration_file {
        db.set_task_metadata_field(&id, "dispatch_worktree_file", &path.to_string_lossy())
            .await
            .expect("set dispatch_worktree_file");
    }
    id
}

/// Drive one full DB-scan cycle (`DB_SCAN_INTERVAL_TICKS` = 60).
async fn drive_scan(engine: &mut TaskEngine) {
    for _ in 0..60 {
        engine.tick().await;
    }
}

async fn stall_audit_count(db: &AsyncDatabase) -> i64 {
    db.count_audit_events_by_tool_name("pilot_silent_stall")
        .await
        .expect("count audit events")
}

// ---------------------------------------------------------------------------
// AC5 — non-vacuity: the fb355061 shape is detected and disposed of.
// ---------------------------------------------------------------------------

/// The founding case, armed: `callback`/`in_progress`, PID alive, worktree
/// declared and idle past the window. Asserts BOTH halves of AC1/AC2 — the
/// status transition and the audit row — plus AC6's `failed`-not-`cancelled`.
#[tokio::test]
async fn stalled_pilot_is_detected_and_disposed() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "long_running:run_claude_pilot — pilote muet",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;

    assert_eq!(
        stall_audit_count(&db).await,
        0,
        "baseline: no pilot_silent_stall audit events before the scan"
    );

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "failed",
        "an armed reaper must transition the stalled dispatch to failed"
    );
    assert_ne!(
        task.status, "cancelled",
        "AC6: `cancelled` carries do-NOT-retry downstream; this dispatch must be re-drivable"
    );
    assert_eq!(
        stall_audit_count(&db).await,
        1,
        "AC2 load-bearing: exactly one pilot_silent_stall audit event per reaped dispatch"
    );

    kill_pid(pid);
}

/// AC6: the discriminator file is written BEFORE the signal, and it is NOT a
/// `CANCELLED_BY_*` value — otherwise `self-dev-callback` reads the reap as an
/// operator cancel and refuses to retry, killing the pilot *and* the retry.
#[tokio::test]
async fn disposition_writes_a_non_cancel_discriminator() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let reason_path = format!("/tmp/mika-cancel-reason-{pid}");
    let _ = std::fs::remove_file(&reason_path);

    seed_dispatch(
        &db,
        "pilote muet — raison",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;
    drive_scan(&mut engine).await;

    let reason = std::fs::read_to_string(&reason_path)
        .expect("the reaper must pre-write the cancel-reason file before SIGTERM");
    assert!(
        reason.contains("REAPED_PILOT_SILENT_STALL"),
        "expected the silent-stall discriminator, got {reason:?}"
    );
    assert!(
        !reason.contains("CANCELLED_BY"),
        "the discriminator must stay outside the CANCELLED_BY_* family (Décision 3), got {reason:?}"
    );

    let _ = std::fs::remove_file(&reason_path);
    kill_pid(pid);
}

// ---------------------------------------------------------------------------
// AC8 — disarmed, it observes without disposing.
// ---------------------------------------------------------------------------

/// Same input as AC5, disposition explicitly disarmed: the audit row proves the
/// detector SAW; the untouched status and the live PID prove it did not FIRE.
/// Positive and negative control in one test, which is the point — an "observes
/// only" claim asserted by absence alone is indistinguishable from a broken
/// detector.
///
/// Named for what it tests since mika#2272. It used to be called
/// `observation_is_the_default`, and it kept passing after the default flipped
/// to armed because it passes `armed: false` explicitly — a green test whose
/// name asserted the opposite of the shipped behaviour.
#[tokio::test]
async fn disarmed_it_observes_without_disposing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    // Explicitly disarmed. Since mika#2272 this is the opt-out, not the
    // default; `mika_common::config` pins the default and
    // `test_reaper_reaps_live_pending_pilot_2272.rs` exercises it.
    let dispatcher = test_dispatcher(db.clone(), false);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "pilote muet — observation",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;

    drive_scan(&mut engine).await;

    assert_eq!(
        stall_audit_count(&db).await,
        1,
        "AC8 positive control: the detector must still write its audit row when disarmed"
    );
    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "in_progress",
        "AC8 negative control: a disarmed reaper must not transition the task"
    );
    assert!(
        mika_agent::task_engine::process_liveness::is_same_process_alive(
            u32::try_from(pid).unwrap(),
            start_time
        ),
        "AC8 negative control: a disarmed reaper must not signal the process"
    );

    kill_pid(pid);
}

// ---------------------------------------------------------------------------
// AC4 — five negative controls, each neutralising exactly ONE term.
// ---------------------------------------------------------------------------

/// (a) The worktree was written recently. Every other term holds — this is the
/// term the 24-minute inter-write gap measured on the healthy run c3f9a2f9
/// would have tripped under the ticket body's illustrative 15–20 min.
#[tokio::test]
async fn recent_worktree_write_is_not_reaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), 5);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "pilote sain — écriture récente",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "in_progress",
        "a working pilot must not be reaped"
    );
    assert_eq!(
        stall_audit_count(&db).await,
        0,
        "and must not even be reported"
    );

    kill_pid(pid);
}

/// (b) The process is dead. That dispatch belongs to
/// `check_callback_process_liveness`, whose grace period and failure reason
/// differ — two reapers claiming one row would double-report it.
#[tokio::test]
async fn dead_process_is_not_this_reapers_population() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_and_reap_child();

    seed_dispatch(
        &db,
        "pilote mort — pas notre population",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;

    drive_scan(&mut engine).await;

    assert_eq!(
        stall_audit_count(&db).await,
        0,
        "a dead process must not produce a pilot_silent_stall row"
    );
}

/// (c) No worktree declared at all — the free-text dispatch shape (mika#1593).
/// Absence of evidence must not become evidence.
#[tokio::test]
async fn undeclared_worktree_is_not_reaped() {
    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "dispatch free-text — sans worktree",
        pid,
        Some(start_time),
        None,
    )
    .await;

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "in_progress",
        "an undeclared dispatch is not a candidate"
    );
    assert_eq!(stall_audit_count(&db).await, 0);

    kill_pid(pid);
}

/// (d) The declaration exists but names a path that does not — a lost or
/// already-cleaned worktree. Same fail-safe as (c), reached by a different
/// route, which is why it is a separate control rather than a variant.
#[tokio::test]
async fn declared_but_missing_worktree_is_not_reaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ghost = tmp.path().join("worktree-that-was-removed");
    let declaration = declare_worktree(tmp.path(), &ghost);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "worktree disparu",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "in_progress",
        "a path that does not exist is not evidence of staleness"
    );
    assert_eq!(stall_audit_count(&db).await, 0);

    kill_pid(pid);
}

/// (e) The task has left the live statuses — the shape of a callback that landed
/// while the scan was doing filesystem I/O. `get_live_dispatch_callback_tasks_with_pid`
/// already filters on `pending`/`in_progress`, so this control pins the SELECT
/// term together with the re-read that follows the scan. `completed` is chosen
/// deliberately: since mika#2272 `pending` is *inside* the population, so a
/// control using it would no longer neutralise this term.
#[tokio::test]
async fn task_no_longer_in_progress_is_not_reaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, start_time) = spawn_live_child();
    let task_id = seed_dispatch(
        &db,
        "callback en vol — déjà livré",
        pid,
        Some(start_time),
        Some(&declaration),
    )
    .await;
    db.update_task_status(&task_id, "completed")
        .await
        .expect("simulate the in-flight callback landing first");

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "completed",
        "the in-flight callback must win the race cleanly"
    );
    assert_eq!(stall_audit_count(&db).await, 0);

    kill_pid(pid);
}

/// Sixth control, not in AC4 but the same class: a PID with no stored
/// `process_start_time`. The pair that identifies a process *instance* is
/// incomplete, so a recycled PID would read as alive — and this reaper kills.
#[tokio::test]
async fn pid_without_start_time_is_not_reaped() {
    let tmp = tempfile::tempdir().expect("tmp");
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let db = test_db();
    let dispatcher = test_dispatcher(db.clone(), true);
    let mut engine = TaskEngine::new(db.clone(), dispatcher);

    let (pid, _start_time) = spawn_live_child();
    let task_id = seed_dispatch(&db, "PID sans start_time", pid, None, Some(&declaration)).await;

    drive_scan(&mut engine).await;

    let task = db.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(
        task.status, "in_progress",
        "without a start time a recycled PID reads as alive — decline rather than guess"
    );
    assert_eq!(stall_audit_count(&db).await, 0);

    kill_pid(pid);
}
