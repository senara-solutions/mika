use anyhow::Result;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;

use super::cron::{
    extract_timezone_from_metadata, next_fire_from_cron, next_fire_from_cron_tz, parse_timezone,
};
use super::dispatcher::TaskDispatcher;
use super::liveness::EngineHeartbeat;
use super::queue::QueuedTask;
use super::types::{action_type, task_status, trigger_type};

/// Maximum tasks fired per tick to prevent overrun when many tasks are overdue.
const MAX_PER_TICK: usize = 10;

/// How many ticks between periodic DB scans for tasks created outside the engine.
const DB_SCAN_INTERVAL_TICKS: u64 = 60;

/// Grace period (seconds) before the reaper transitions an orphaned parent
/// self_dev task to `failed`. 600s ≈ 3× the upper bound of observed callback
/// duration (mika#868 audit: 187s LLM latency). Long enough for #870's
/// re-enter recovery to complete; short enough that the operator's dispatch
/// queue clears within one tick-cycle after grace expires. See #871.
const REAPER_GRACE_SECONDS: i64 = 600;

/// Default grace window (seconds) before the childless-parent reaper transitions
/// a self_dev issue parent left `in_progress` with **zero** callback children to
/// `failed` (mika#1687). Deliberately far larger than [`REAPER_GRACE_SECONDS`]
/// (600): a legitimately-dispatching parent is childless only for the sub-second
/// window between its `pending → in_progress` transition and the callback child
/// row commit inside `spawn_long_running_exec()`. 30 min removes any plausible
/// in-flight false positive (the ticket's real cases were 100–180 min old).
/// Overridable via `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS`.
const CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS: i64 = 1800;

/// Env var overriding [`CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS`].
const CHILDLESS_PARENT_REAPER_GRACE_ENV: &str = "MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS";

/// Ticks between pilot-transcript retention sweeps (mika#1705 AC6). At the 1s
/// tick cadence, 86_400 ticks ≈ 24h — a daily prune, matching the plan's
/// "daily tick deletes rows older than N days". Startup also runs one sweep.
const PILOT_TRANSCRIPT_RETENTION_INTERVAL_TICKS: u64 = 86_400;

/// Env var overriding the pilot-transcript retention window (mika#1705 AC6).
const PILOT_TRANSCRIPT_RETENTION_ENV: &str = "MIKA_PILOT_TRANSCRIPT_RETENTION_DAYS";

/// Default pilot-transcript retention window in days (mika#1705 AC6).
const PILOT_TRANSCRIPT_RETENTION_DEFAULT_DAYS: i64 = 90;

/// The two currently-defined dispatch classes (per `derive_dispatch_class` at
/// `skills/executor.rs`). Iteration order is `implement` first because pre-v34
/// NULL-class wrappers fall into this bucket via `COALESCE` — promote those
/// before grooming wrappers when both classes are idle. Ordering is cosmetic —
/// both classes process independently in a single tick. The `Test 5` shape
/// test in this crate pins this slice against `derive_dispatch_class` to catch
/// drift when a new class is added to the executor (mika#1175).
const DISPATCH_CLASSES: &[&str] = &["implement", "groom"];

/// Parse one claude-pilot transcript JSONL object into a [`crate::db::PilotTranscriptRow`]
/// (mika#1705). Missing fields become `None`; body fields are secret-scrubbed.
/// Body values that are JSON objects/arrays are serialized to their compact
/// string form before scrubbing (claude-pilot may emit either shape).
fn parse_pilot_transcript_line(v: &serde_json::Value) -> crate::db::PilotTranscriptRow {
    let str_field = |key: &str| v.get(key).and_then(|x| x.as_str()).map(str::to_owned);
    let i64_field = |key: &str| v.get(key).and_then(serde_json::Value::as_i64);
    let scrubbed_body = |key: &str| -> Option<String> {
        match v.get(key) {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => {
                Some(crate::secret_scrubber::scrub_secrets(s).into_owned())
            }
            Some(other) => {
                Some(crate::secret_scrubber::scrub_secrets(&other.to_string()).into_owned())
            }
        }
    };
    crate::db::PilotTranscriptRow {
        timestamp: str_field("timestamp"),
        provider: str_field("provider"),
        model: str_field("model"),
        request_body: scrubbed_body("request_body"),
        response_body: scrubbed_body("response_body"),
        tokens_in: i64_field("tokens_in"),
        tokens_out: i64_field("tokens_out"),
        latency_ms: i64_field("latency_ms"),
    }
}

/// The unified task engine: a min-heap BinaryHeap backed by SQLite, driven by a
/// 1-second tick loop that fires tasks whose `next_fire_at <= now`.
///
/// Replaces `ReminderScheduler`. All proactive behaviors (reminders, heartbeat,
/// reflection, team delegation) route through this engine.
///
/// # Concurrency model
///
/// The engine is wrapped in `Arc<Mutex<TaskEngine>>`. The tick loop acquires the
/// mutex for a short window to drain the heap and call `fire_task()`. Each
/// `fire_task()` immediately `tokio::spawn`s the heavy dispatch work, releasing
/// the mutex before any I/O. This prevents the lock from blocking user message
/// processing.
pub struct TaskEngine {
    db: AsyncDatabase,
    queue: BinaryHeap<QueuedTask>,
    /// Task IDs currently in the heap (prevents duplicates on periodic DB scan).
    queued_ids: HashSet<String>,
    dispatcher: Arc<TaskDispatcher>,
    reenqueue_tx: mpsc::Sender<QueuedTask>,
    reenqueue_rx: mpsc::Receiver<QueuedTask>,
    /// Tick counter used to trigger periodic DB scans.
    tick_count: u64,
    /// Wedge-watchdog heartbeat (mika#1850). Updated at the top of every
    /// tick; read by [`super::liveness::spawn_engine_wedge_watchdog`]
    /// on its own cadence to detect wedged tick loops.
    heartbeat: EngineHeartbeat,
}

impl TaskEngine {
    pub fn new(db: AsyncDatabase, dispatcher: Arc<TaskDispatcher>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            db,
            queue: BinaryHeap::new(),
            queued_ids: HashSet::new(),
            dispatcher,
            reenqueue_tx: tx,
            reenqueue_rx: rx,
            tick_count: 0,
            heartbeat: EngineHeartbeat::new(),
        }
    }

    /// Clone the wedge-watchdog heartbeat handle (mika#1850). Cheap — the
    /// underlying state is an `Arc<AtomicI64>`. Called by the server startup
    /// after engine construction to hand a shared handle to
    /// [`super::liveness::spawn_engine_wedge_watchdog`].
    pub fn heartbeat(&self) -> EngineHeartbeat {
        self.heartbeat.clone()
    }

    /// Called at startup.
    ///
    /// 1. Expires tasks past their `timeout_at`.
    /// 2. Marks orphaned `in_progress` tasks as `failed` (no process survived restart).
    /// 3. Loads `pending` and `recurring_active` tasks into the `BinaryHeap`.
    ///
    /// Returns `(loaded_count, queue_len)` for the caller to aggregate across agents.
    pub async fn startup_recovery(&mut self) -> Result<(usize, usize)> {
        let now = crate::timestamp::now();

        // 1. Expire timed-out tasks
        match self.db.mark_tasks_expired(&now).await {
            Ok(n) if n > 0 => info!(count = n, "expired timed-out tasks on startup"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to expire timed-out tasks"),
        }

        // 1b. Kill orphan processes for newly expired tasks
        self.kill_orphan_processes().await;

        // 2. Recover in_progress tasks (process couldn't have survived container restart)
        let in_progress = self
            .db
            .get_tasks_by_status(vec![task_status::IN_PROGRESS.to_string()])
            .await
            .unwrap_or_default();

        for task in in_progress {
            // Manual (task) tasks represent human work — don't invalidate on restart.
            //
            // NARROWED (mika#1712 step 2b, 2026-08-21): the sibling
            // `sweep_null_pid_phantoms_at_startup()` below DOES transition a
            // subset of manual tasks — those matching the phantom shape
            // (`action_type='none'` + `process_id IS NULL` + status in
            // `('in_progress','blocked')`). That is the leak class from
            // plan §7 D2 and is intentionally excluded from this
            // manual-preservation guard. The guard here still protects the
            // rest of the manual task surface (any manual row with a real
            // action_type or non-NULL process_id).
            if task.trigger_type == trigger_type::MANUAL {
                debug!(task_id = %task.id, "skipping manual task during startup recovery");
                continue;
            }
            debug!(task_id = %task.id, "marking orphaned in_progress task as failed on startup");
            if let Err(e) = self
                .db
                .update_task_status(&task.id, task_status::FAILED)
                .await
            {
                warn!(task_id = %task.id, error = %e, "failed to mark task as failed during recovery");
            }
        }

        // 2b. Sweep pre-existing NULL-PID phantom tracking rows (mika#1712 AC5).
        // Any phantom row present at startup outlived a prior server process —
        // that is exactly the "startup sweep" AC5 wants. `age_seconds=0`
        // matches every candidate regardless of freshness (SQLite treats
        // `strftime('now', '-0 seconds')` as "now", so `updated_at < now`
        // selects any past row). SOLE WRITER: `phantom_aged_out` audit
        // tool_name is shared with the AC3 tick sweep in
        // `sweep_null_pid_phantoms`; the `reasoning` field carries the source
        // discriminator (`startup_sweep` here).
        self.sweep_null_pid_phantoms_at_startup().await;

        // 3. Load schedulable tasks into BinaryHeap
        let schedulable = self.db.get_schedulable_tasks().await.unwrap_or_default();
        let count = schedulable.len();

        for task in schedulable {
            self.enqueue_queued_task(
                &task.id,
                &task.trigger_type,
                &task.action_type,
                task.cron_expr.as_deref(),
                task.next_fire_at.as_deref(),
                &now,
                task.metadata.as_deref(),
            );
        }

        // 4. Prune ended system/silent sessions older than 7 days
        const SEVEN_DAYS_SECS: i64 = 7 * 24 * 60 * 60;
        match self.db.prune_old_sessions(SEVEN_DAYS_SECS).await {
            Ok(n) if n > 0 => info!(count = n, "pruned old ended sessions on startup"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to prune old sessions"),
        }

        // 5. Prune LLM call and tool call records older than 30 days
        const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
        match self.db.prune_old_llm_calls(THIRTY_DAYS_SECS).await {
            Ok(n) if n > 0 => info!(count = n, "pruned old llm_calls on startup"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to prune old llm_calls"),
        }
        match self.db.prune_old_tool_calls(THIRTY_DAYS_SECS).await {
            Ok(n) if n > 0 => info!(count = n, "pruned old tool_calls on startup"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to prune old tool_calls"),
        }

        // 6. Prune pilot transcripts past the retention window (mika#1705 AC6).
        self.prune_old_pilot_transcripts().await;

        debug!(
            loaded = count,
            queue_len = self.queue.len(),
            "task engine startup recovery complete"
        );
        Ok((count, self.queue.len()))
    }

    /// Insert a new task into the DB and enqueue it in the BinaryHeap.
    ///
    /// Returns the new task's UUID string ID.
    pub async fn enqueue(&mut self, task: NewTask) -> Result<String> {
        let next_fire_at = task.next_fire_at.clone();
        let trigger_type_val = task.trigger_type.clone();
        let action_type_val = task.action_type.clone();
        let cron_expr = task.cron_expr.as_deref().map(str::to_owned);

        let id = self.db.create_task(task).await?;

        if let Some(fire_at) = next_fire_at {
            self.push_to_heap(QueuedTask {
                task_id: id.clone(),
                next_fire_at: fire_at,
                trigger_type: trigger_type_val,
                action_type: action_type_val,
                cron_expr,
            });
        }

        Ok(id)
    }

    /// Cancel a task in the DB.
    ///
    /// The task's entry in the BinaryHeap will be skipped gracefully when it
    /// reaches the top, because `fire_task` checks DB status before marking
    /// in_progress.
    pub async fn cancel(&self, task_id: &str) -> Result<bool> {
        self.db.cancel_task(task_id).await
    }

    /// Spawn the 1-second tick loop as a background task.
    ///
    /// The returned `JoinHandle` can be aborted on shutdown.
    pub fn spawn_tick_loop(engine: Arc<Mutex<Self>>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                let mut eng = engine.lock().await;
                eng.tick().await;
                // Lock released here. Heavy dispatch runs in spawned tasks.
            }
        })
    }

    /// One tick: drain the re-enqueue channel, run periodic DB scan, then fire
    /// all due tasks (up to MAX_PER_TICK).
    ///
    /// Visibility: `pub` so integration tests can drive the tick pipeline
    /// (e.g. mika#1712's `tests/eval/test_phantom_task_row_sweep.rs` exercises
    /// the sweep-from-tick wiring end-to-end). Production callers still go
    /// through [`Self::spawn_tick_loop`].
    pub async fn tick(&mut self) {
        // Update wedge-watchdog heartbeat (mika#1850) — MUST be first line of
        // tick body so a wedge in downstream awaits still tells the watchdog
        // "the previous tick reached this point at time T". If we placed it
        // after the DB scans, a hung scan would look like the loop never
        // ticked at all.
        self.heartbeat.tick();

        // Drain re-enqueue messages from completed recurring tasks
        while let Ok(task) = self.reenqueue_rx.try_recv() {
            self.push_to_heap(task);
        }

        // Periodic DB scan: pick up tasks created outside the engine (e.g. by tools)
        // and expire timed-out tasks (long_running callbacks past their deadline).
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS) {
            self.expire_timed_out_tasks().await;
            self.kill_orphan_processes().await;
            self.check_callback_process_liveness().await;
            // mika#1712: sweep NULL-PID phantom tracking rows the callback
            // watchdog cannot see (its first predicate is
            // `process_id IS NOT NULL`) and the orphaned-parent reaper does not
            // match (no `resume_agent` callback child). Runs inside the same
            // 60-tick cadence — no new interval.
            self.sweep_null_pid_phantoms().await;
            self.scan_db_for_new_tasks().await;
            // In CLI mode, the TUI's poll_callback_tasks() handles callback delivery
            // (session-scoped, atomic claim). Skip engine dispatch to prevent a race
            // where the engine steals callbacks and processes them in a context-free
            // silent turn. See #264.
            if !self.dispatcher.cli_mode {
                // mika#1070 — Promote-first ordering: promote deferred wrappers
                // before scanning for dispatchable callbacks. The promotion DB write
                // commits synchronously, so a promoted wrapper is visible to the
                // dispatch_undelivered_callbacks scan in the same tick cycle.
                self.promote_pending_deferred_if_idle().await;
                self.dispatch_undelivered_callbacks().await;
            }
            // Reap parent self_dev tasks left in_progress after their callback
            // subtask delivered without producing a PR (#871).
            self.reap_orphaned_parent_tasks().await;

            // Auto-complete parent self_dev tasks left in_progress after their
            // callback subtask delivered WITH a PR url (mika#1162). Success-side
            // sibling to the reaper: catches crash-recovery cases and pre-deploy
            // wedges that the inline path in `dispatch_resume_agent` can't reach.
            self.complete_parent_tasks_on_callback_success().await;

            // Reap parent self_dev issue tasks left in_progress with ZERO
            // callback children, aged past the childless grace window (mika#1687).
            // The silent-pilot-death backstop: a parent that reached in_progress
            // without ever recording a callback child falls through both reapers
            // above (they INNER-JOIN a delivered child) and the watchdog (it keys
            // off a callback child's PID). Runs AFTER the completer so any
            // delivered-child success/failure case resolves first and only
            // genuinely childless parents reach here.
            self.reap_childless_stuck_parent_tasks().await;

            // Reap orphaned team runs left in `status='running'` when no
            // terminal-state writer ran (mika#1652). Failure-path sibling of
            // the parent-task reaper above, for the `team_runs` lifecycle:
            // frees team slots held by runs whose finalizer never executed.
            crate::teams::engine::reap_orphaned_team_runs(&self.db).await;

            // mika#1705: ingest finished claude-pilot transcript JSONL files
            // into the pilot_transcripts table (the implementation-reasoning
            // corpus for the owned-model bet).
            self.ingest_pilot_transcripts().await;
        }

        // mika#1705 AC6: daily pilot-transcript retention sweep.
        if self
            .tick_count
            .is_multiple_of(PILOT_TRANSCRIPT_RETENTION_INTERVAL_TICKS)
        {
            self.prune_old_pilot_transcripts().await;
        }

        let now = crate::timestamp::now();
        let mut fired = 0;

        while fired < MAX_PER_TICK {
            match self.queue.peek() {
                Some(t) if t.next_fire_at <= now => {
                    let task = self.pop_from_heap().unwrap();
                    self.fire_task(task).await;
                    fired += 1;
                }
                _ => break,
            }
        }

        if fired > 0 {
            debug!(fired, "tick fired tasks");
        }
    }

    /// Expire tasks past their `timeout_at` deadline.
    ///
    /// Runs periodically (every `DB_SCAN_INTERVAL_TICKS` seconds) to catch
    /// long_running callback tasks that never completed. After expiring, checks
    /// sibling completion so parent tasks can fire even when a child times out.
    async fn expire_timed_out_tasks(&mut self) {
        let now = crate::timestamp::now();
        match self.db.mark_tasks_expired(&now).await {
            Ok(n) if n > 0 => {
                info!(count = n, "expired timed-out tasks in tick loop");
                // Check if any expired tasks unblock their parent
                self.check_expired_siblings().await;
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to expire timed-out tasks in tick loop"),
        }
    }

    /// After expiring tasks, check if any parent tasks now have all children
    /// in terminal states (completed/failed/expired/cancelled).
    async fn check_expired_siblings(&self) {
        let expired_ids = self
            .db
            .get_expired_child_task_ids()
            .await
            .unwrap_or_default();

        for task_id in expired_ids {
            let d = self.dispatcher.clone();
            let tid = task_id.clone();
            tokio::spawn(async move {
                d.check_and_dispatch_parent(&tid).await;
            });
        }
    }

    /// Send SIGTERM to orphan processes for expired tasks that still have a process_id set,
    /// then clear the process_id to prevent repeated kill attempts.
    async fn kill_orphan_processes(&self) {
        match self.db.get_expired_tasks_with_process_id().await {
            Ok(tasks) => {
                for (task_id, pid) in tasks {
                    info!(task_id = %task_id, pid = pid, "killing orphan process for expired task");
                    // Orphan cleanup: pass `None` for expected_start_time. The
                    // expired-task query returns (task_id, pid) only; we don't
                    // wire the metadata extraction here because orphan cleanup
                    // is best-effort and the existing /proc/<pid>/stat existence
                    // check is good enough for this path. The PID reuse guard
                    // (#855) is targeted at the cancel-by-operator path where
                    // wrong-process-kill consequences are higher.
                    super::process_kill::kill_process_immediate(pid, None);
                    // Clear process_id so we don't attempt to kill again on next tick
                    if let Err(e) = self.db.clear_task_process_id(&task_id).await {
                        warn!(task_id = %task_id, error = %e, "failed to clear process_id after kill");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to query expired tasks with process_id");
            }
        }
    }

    /// Scan DB for schedulable tasks not already in the heap.
    /// Called every `DB_SCAN_INTERVAL_TICKS` seconds as a safety net.
    async fn scan_db_for_new_tasks(&mut self) {
        let now = crate::timestamp::now();
        let tasks = match self.db.get_schedulable_tasks().await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "periodic task scan failed");
                return;
            }
        };
        let mut added = 0;
        for task in tasks {
            if self.queued_ids.contains(&task.id) {
                continue;
            }
            self.enqueue_queued_task(
                &task.id,
                &task.trigger_type,
                &task.action_type,
                task.cron_expr.as_deref(),
                task.next_fire_at.as_deref(),
                &now,
                task.metadata.as_deref(),
            );
            added += 1;
        }
        if added > 0 {
            debug!(added, "periodic scan added new tasks to engine queue");
        }
    }

    /// Dispatch undelivered callback tasks (both completed and failed) directly
    /// to the agent. In server mode, failed callbacks from the background monitor
    /// have no external trigger — this periodic scan ensures they are delivered.
    ///
    /// Unlike schedulable tasks, callbacks bypass the heap and dispatch immediately
    /// because they are already in a terminal state waiting for delivery.
    /// `dispatch_resume_agent` handles the atomic `mark_task_delivered` call internally.
    async fn dispatch_undelivered_callbacks(&self) {
        let since = crate::timestamp::now_minus(chrono::Duration::days(7));
        let tasks = match self.db.get_undelivered_callback_tasks(&since).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "failed to scan for undelivered callback tasks");
                return;
            }
        };

        let stale_threshold =
            chrono::Duration::minutes(crate::planning::policy::STALE_FAILED_CALLBACK_MINUTES);
        let mut stale_skipped: usize = 0;

        let now = crate::timestamp::now();
        for task in tasks {
            // Retry delay guard: skip tasks whose next_fire_at is in the future.
            // AgentBusy recovery (mika#1070) keeps status as 'completed' but sets
            // next_fire_at to enforce a 30s retry delay.
            if let Some(ref fire_at) = task.next_fire_at
                && fire_at.as_str() > now.as_str()
            {
                continue;
            }

            // Staleness guard: skip failed callbacks older than the threshold.
            // Completed callbacks are always delivered — they may carry legitimate results.
            if task.status == "failed" {
                let is_stale = task
                    .completed_at
                    .as_deref()
                    .is_some_and(|ts| crate::timestamp::is_older_than(ts, stale_threshold));
                if is_stale {
                    if let Ok(true) = self.db.mark_task_delivered(&task.id).await {
                        stale_skipped += 1;
                        debug!(
                            task_id = %task.id,
                            label = %task.label,
                            "skipped stale failed callback"
                        );
                    }
                    continue;
                }
            }

            let dispatcher = self.dispatcher.clone();
            tokio::spawn(async move {
                if let Err(e) = dispatcher.dispatch_resume_agent(&task).await {
                    // AgentBusy is expected — next scan cycle will retry
                    if !matches!(e, super::dispatcher::DispatchError::AgentBusy(_)) {
                        warn!(task_id = %task.id, error = %e, "failed to dispatch undelivered callback");
                    }
                }
            });
        }

        if stale_skipped > 0 {
            info!(count = stale_skipped, "cleared stale failed callback tasks");
        }
    }

    /// Engine-level backstop for deferred-dispatch promotion (mika#1070, mika#1175).
    ///
    /// Runs every `DB_SCAN_INTERVAL_TICKS`. For each `dispatch_class`, if pending
    /// deferred wrappers of that class exist AND no active non-deferred callback
    /// exists in that class, promotes the oldest wrapper of that class. Per-class
    /// iteration (mika#1175) prevents cross-class throughput halving when wrappers
    /// from multiple classes are pending. This recovers from any scenario where
    /// the inline promotion at `dispatch_resume_agent` (dispatcher.rs) fails to fire.
    async fn promote_pending_deferred_if_idle(&self) {
        for class in DISPATCH_CLASSES {
            match self.db.has_any_active_callback_for_class(class).await {
                Ok(true) => continue, // Class slot occupied — skip this class
                Ok(false) => {}       // Class slot free — try to promote one wrapper
                Err(e) => {
                    warn!(
                        error = %e,
                        dispatch_class = class,
                        "failed to check active callbacks for deferred promotion"
                    );
                    continue; // Fail-closed for this class — try the others
                }
            }
            self.dispatcher
                .dispatch_next_deferred_callback_for_class(class)
                .await;
        }
    }

    /// Detect dead subprocesses for active callback tasks (#959).
    ///
    /// For each `in_progress` callback task with a `process_id`:
    /// 1. Check if the process is still alive (PID exists AND start time matches)
    /// 2. If dead, set `first_dead_at` metadata on first detection
    /// 3. If dead and past grace period, mark the task `failed`
    /// 4. If alive, clear any stale `first_dead_at` (defensive)
    ///
    /// Grace period default: 120s (configurable via `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`).
    async fn check_callback_process_liveness(&self) {
        let tasks = match self.db.get_active_callback_tasks_with_pid().await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "callback_watchdog: failed to query active callback tasks");
                return;
            }
        };

        if tasks.is_empty() {
            return;
        }

        let grace_period_secs = self
            .dispatcher
            .settings
            .effective_callback_watchdog_grace_period_secs();

        for task in tasks {
            let pid = match task.process_id {
                Some(pid) if pid > 0 => pid as u32,
                _ => continue,
            };

            // Read stored process_start_time from task metadata
            let start_time: Option<u64> = task
                .metadata
                .as_deref()
                .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                .and_then(|v| v.get("process_start_time")?.as_str()?.parse().ok());

            let process_alive = match start_time {
                Some(st) => super::process_liveness::is_same_process_alive(pid, st),
                // No start time stored (pre-#959 task or non-Linux) — fall back to
                // basic /proc check. Less reliable but better than nothing.
                None => {
                    #[cfg(target_os = "linux")]
                    {
                        std::path::Path::new(&format!("/proc/{pid}")).exists()
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        true // Assume alive on non-Linux — existing timeout_at is the fallback
                    }
                }
            };

            if process_alive {
                // Process is alive — clear any stale first_dead_at (defensive)
                let has_first_dead_at = task
                    .metadata
                    .as_deref()
                    .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                    .and_then(|v| v.get("first_dead_at").cloned())
                    .is_some();

                if has_first_dead_at {
                    let _ = self
                        .db
                        .remove_task_metadata_field(&task.id, "first_dead_at")
                        .await;
                }
                continue;
            }

            // Process is dead — check grace period
            let first_dead_at = task
                .metadata
                .as_deref()
                .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                .and_then(|v| v.get("first_dead_at")?.as_str().map(|s| s.to_string()));

            let now = crate::timestamp::now();

            match first_dead_at {
                None => {
                    // First detection — record timestamp
                    debug!(
                        task_id = %task.id,
                        pid = pid,
                        "callback_watchdog: subprocess PID dead, starting grace period"
                    );
                    let _ = self
                        .db
                        .set_task_metadata_field(&task.id, "first_dead_at", &now)
                        .await;
                }
                Some(first_dead) => {
                    // Check if grace period has elapsed
                    let grace_duration = chrono::Duration::seconds(grace_period_secs as i64);
                    if !crate::timestamp::is_older_than(&first_dead, grace_duration) {
                        // Still within grace period — wait
                        debug!(
                            task_id = %task.id,
                            pid = pid,
                            first_dead_at = %first_dead,
                            grace_period_secs = grace_period_secs,
                            "callback_watchdog: still within grace period"
                        );
                        continue;
                    }

                    // Grace period elapsed — re-check task status to guard against
                    // race with in-flight callback delivery
                    let current_task = match self.db.get_task(&task.id).await {
                        Ok(Some(t)) => t,
                        _ => continue,
                    };

                    if current_task.status != "in_progress" {
                        // Task already transitioned (callback delivered during grace)
                        debug!(
                            task_id = %task.id,
                            status = %current_task.status,
                            "callback_watchdog: task already transitioned, skipping"
                        );
                        continue;
                    }

                    // Mark task as failed
                    match self
                        .db
                        .update_task_failed(&task.id, "subprocess_exited_without_delivery")
                        .await
                    {
                        Ok(true) => {
                            // Clear timeout_at to prevent double-processing by
                            // expire_timed_out_tasks
                            let _ = self
                                .db
                                .set_task_metadata_field(
                                    &task.id,
                                    "watchdog_cleared_timeout",
                                    "true",
                                )
                                .await;

                            warn!(
                                task_id = %task.id,
                                parent_task_id = ?task.parent_task_id,
                                pid = pid,
                                process_start_time = ?start_time,
                                first_dead_at = %first_dead,
                                grace_period_secs = grace_period_secs,
                                failure_reason = "subprocess_exited_without_delivery",
                                "callback_watchdog_detected_process_death: \
                                 subprocess exited without delivering callback, \
                                 marking task failed to unblock dispatch queue"
                            );
                        }
                        Ok(false) => {
                            debug!(
                                task_id = %task.id,
                                "callback_watchdog: task already in terminal state"
                            );
                        }
                        Err(e) => {
                            warn!(
                                task_id = %task.id,
                                error = %e,
                                "callback_watchdog: failed to mark task as failed"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Sweep NULL-PID phantom tracking rows (mika#1712, AC3).
    ///
    /// Selects rows with `action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`, and `updated_at` older than the
    /// configured grace window (`MIKA_PHANTOM_SWEEP_AGE_SECONDS`, default
    /// 3600s). Transitions each match to `failed` with `error_reason =
    /// "phantom_aged_out"` via `update_task_failed` (guarded UPDATE — races
    /// with in-flight operator/agent transitions lose cleanly).
    ///
    /// Per-row emits an `audit_events` row with `tool_name='phantom_aged_out'`
    /// carrying the pre-sweep status in `before_value`, `"failed"` in
    /// `after_value`, and a source-discriminating `reasoning` field so the
    /// AC3 (watchdog) and AC5 (startup) branches can be joined offline.
    ///
    /// Per-pass emits a `phantom_sweep_complete` INFO log line when at least
    /// one row was swept, with `source="watchdog_tick"` and the aggregate
    /// count. On count > 100 additionally emits `phantom_sweep_large_backlog`
    /// WARN for operator anomaly visibility. NEVER caps the sweep — the
    /// telemetry is the point (feeds the mika#1934 cause-racine investigation
    /// per sami bearing §3 "no silent cap").
    ///
    /// SOLE WRITER: phantom_aged_out — this method (AC3 tick source) and the
    /// startup step 2b inside [`Self::startup_recovery`] (AC5) are the only
    /// two sites that write the `phantom_aged_out` audit tool_name. The
    /// `reasoning` field carries the source discriminator; the `tool_name`
    /// stays constant so the audit-events query surface is a single predicate
    /// (`WHERE tool_name='phantom_aged_out'`) rather than a two-branch union.
    ///
    /// SOLE WRITER: phantom_sweep_db_error — the same two sites also emit the
    /// distinct `phantom_sweep_db_error` audit tool_name when
    /// `update_task_failed` returns Err. Keeping the error branch on its own
    /// tool_name preserves the AC7 count semantics of
    /// `SELECT COUNT(*) FROM audit_events WHERE tool_name='phantom_aged_out'`
    /// (successful transitions only) — the sibling
    /// `SELECT ... WHERE tool_name='phantom_sweep_db_error'` counts failures
    /// separately. Addresses adversarial-reviewer ADV-3 (2026-08-21).
    async fn sweep_null_pid_phantoms(&self) {
        let age_seconds = self
            .dispatcher
            .settings
            .effective_phantom_sweep_age_seconds() as i64;
        let phantoms = match self.db.find_phantom_tracking_tasks(age_seconds).await {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "phantom_sweep: failed to query phantom tracking tasks");
                return;
            }
        };

        if phantoms.is_empty() {
            return;
        }

        let trace_id = mika_common::trace::generate_trace_id();
        let agent_id = self.db.agent_id().to_string();
        let system_session = format!("system-{agent_id}");
        let mut swept_count: u32 = 0;
        let mut error_count: u32 = 0;

        for row in phantoms {
            // ADV-5 (2026-08-21): re-arm heartbeat every row so a large sweep
            // pass never trips the 300s wedge watchdog. Cheap AtomicI64 store.
            self.heartbeat.tick();

            match self
                .db
                .update_task_failed(&row.id, "phantom_aged_out")
                .await
            {
                Ok(true) => {
                    // ADV-4 (2026-08-21): audit-write FIRST, then increment
                    // swept_count only on Ok. This aligns the per-pass count in
                    // the phantom_sweep_complete log with the audit_events row
                    // count — the two AC7 surfaces stay reconcilable.
                    match self
                        .db
                        .log_audit_event(
                            &system_session,
                            "phantom_aged_out",
                            &format!("task:{}", row.id),
                            Some(&row.status),
                            Some("failed"),
                            Some(
                                "phantom_aged_out: manual/none row with NULL process_id \
                                 aged past watchdog grace",
                            ),
                            Some(&trace_id),
                        )
                        .await
                    {
                        Ok(()) => swept_count = swept_count.saturating_add(1),
                        Err(e) => {
                            error_count = error_count.saturating_add(1);
                            warn!(
                                task_id = %row.id,
                                error = %e,
                                "phantom_sweep: failed to write audit event (transition succeeded)"
                            );
                        }
                    }
                }
                Ok(false) => {
                    debug!(
                        task_id = %row.id,
                        "phantom_sweep: task already in terminal state, skipping"
                    );
                }
                Err(e) => {
                    // ADV-3 (2026-08-21): distinct tool_name for the error
                    // branch so operator SQL counting swept rows via
                    // `WHERE tool_name='phantom_aged_out'` is not polluted by
                    // DB failures. Failures are countable separately via
                    // `WHERE tool_name='phantom_sweep_db_error'`.
                    error_count = error_count.saturating_add(1);
                    let _ = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "phantom_sweep_db_error",
                            &format!("task:{}", row.id),
                            Some(&row.status),
                            None,
                            Some(&format!("phantom_sweep_db_error: {e}")),
                            Some(&trace_id),
                        )
                        .await;
                    warn!(
                        task_id = %row.id,
                        error = %e,
                        "phantom_sweep: db error during transition"
                    );
                }
            }
        }

        // R7/ADV-4 (2026-08-21): emit the aggregate line when EITHER
        // successful sweeps OR errors occurred, so a silent-failure pass
        // (many rows queried, all update_task_failed Err) still surfaces.
        if swept_count > 0 || error_count > 0 {
            info!(
                event = "phantom_sweep_complete",
                source = "watchdog_tick",
                count = swept_count,
                error_count = error_count,
                agent_id = %agent_id,
                trace_id = %trace_id,
                "phantom_sweep watchdog tick swept phantom tracking rows"
            );
        }
        if swept_count > 100 {
            warn!(
                event = "phantom_sweep_large_backlog",
                source = "watchdog_tick",
                count = swept_count,
                agent_id = %agent_id,
                "phantom_sweep_large_backlog: single pass swept > 100 rows — anomalous state"
            );
        }
    }

    /// Startup step 2b (mika#1712 AC5): sweep pre-existing NULL-PID phantom
    /// tracking rows at boot with `age_seconds=0` (matches every candidate
    /// regardless of freshness — any phantom present at startup outlived a
    /// prior process by definition). Same per-row transition + audit-event
    /// shape as [`Self::sweep_null_pid_phantoms`] (AC3); the `reasoning` field
    /// carries `"startup_sweep"` so the AC7 telemetry can join the two
    /// branches offline via `SELECT ... WHERE tool_name='phantom_aged_out'`.
    ///
    /// SOLE WRITER coupling: this method and `sweep_null_pid_phantoms` are
    /// the only two writers of the `phantom_aged_out` audit tool_name. The
    /// per-pass telemetry line uses `source="startup_sweep"` to distinguish
    /// from the watchdog tick.
    ///
    /// **Deliberate divergence from step 2's manual-preservation invariant
    /// (ADV-1, 2026-08-21).** Step 2 of [`Self::startup_recovery`] explicitly
    /// skips `trigger_type == MANUAL` in-progress rows with the comment
    /// "Manual (task) tasks represent human work — don't invalidate on
    /// restart". This method narrows that invariant: it DOES transition
    /// manual/none/NULL-PID rows because per plan §7 D2 they are the phantom
    /// shape by design — a legitimate long-running manual tracking row would
    /// have `updated_at` bumped by any `update_task_status` write within
    /// `MIKA_PHANTOM_SWEEP_AGE_SECONDS` (default 3600s), while a genuinely
    /// wedged one has stale `updated_at` far past grace. AC5 at age=0 is
    /// aggressive by intent: any phantom-shape row present at startup
    /// outlived a prior process, which is the ticket's founding-incident
    /// signal (24 rows/18h). If a legitimate multi-hour tracking row gets
    /// swept, the operator un-fails it via SQL (see plan §Rollback).
    async fn sweep_null_pid_phantoms_at_startup(&self) {
        let phantoms = match self.db.find_phantom_tracking_tasks(0).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    error = %e,
                    "phantom_sweep startup: failed to query phantom tracking tasks"
                );
                return;
            }
        };

        if phantoms.is_empty() {
            return;
        }

        let trace_id = mika_common::trace::generate_trace_id();
        let agent_id = self.db.agent_id().to_string();
        let system_session = format!("system-{agent_id}");
        let mut swept_count: u32 = 0;
        let mut error_count: u32 = 0;

        for row in phantoms {
            // ADV-5 (2026-08-21): re-arm heartbeat every row so a large
            // startup pass never trips the wedge watchdog (300s threshold).
            // Watchdog isn't spawned yet at this point in startup_recovery,
            // but the heartbeat.tick() call is cheap AtomicI64 and futureproofs
            // against re-ordering.
            self.heartbeat.tick();

            match self.db.update_task_failed(&row.id, "startup_sweep").await {
                Ok(true) => {
                    // ADV-4 (2026-08-21): audit-write FIRST, then increment on Ok.
                    match self
                        .db
                        .log_audit_event(
                            &system_session,
                            "phantom_aged_out",
                            &format!("task:{}", row.id),
                            Some(&row.status),
                            Some("failed"),
                            Some("startup_sweep: pre-existing phantom found at boot"),
                            Some(&trace_id),
                        )
                        .await
                    {
                        Ok(()) => swept_count = swept_count.saturating_add(1),
                        Err(e) => {
                            error_count = error_count.saturating_add(1);
                            warn!(
                                task_id = %row.id,
                                error = %e,
                                "phantom_sweep startup: failed to write audit event \
                                 (transition succeeded)"
                            );
                        }
                    }
                }
                Ok(false) => {
                    debug!(
                        task_id = %row.id,
                        "phantom_sweep startup: task already in terminal state, skipping"
                    );
                }
                Err(e) => {
                    // ADV-3 (2026-08-21): distinct tool_name for the error branch.
                    error_count = error_count.saturating_add(1);
                    let _ = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "phantom_sweep_db_error",
                            &format!("task:{}", row.id),
                            Some(&row.status),
                            None,
                            Some(&format!("phantom_sweep_db_error: {e}")),
                            Some(&trace_id),
                        )
                        .await;
                    warn!(
                        task_id = %row.id,
                        error = %e,
                        "phantom_sweep startup: db error during transition"
                    );
                }
            }
        }

        if swept_count > 0 || error_count > 0 {
            info!(
                event = "phantom_sweep_complete",
                source = "startup_sweep",
                count = swept_count,
                error_count = error_count,
                agent_id = %agent_id,
                reason = "phantom_signature_null_pid_manual_none",
                trace_id = %trace_id,
                "phantom_sweep startup swept pre-existing phantom rows"
            );
        }
        if swept_count > 100 {
            warn!(
                event = "phantom_sweep_large_backlog",
                source = "startup_sweep",
                count = swept_count,
                agent_id = %agent_id,
                "phantom_sweep_large_backlog: single pass swept > 100 rows — anomalous state"
            );
        }
    }

    /// Reap parent self_dev tasks left `in_progress` after their callback
    /// subtask delivered without producing a PR (#871).
    ///
    /// Transitions matched parents to `failed` with an audit-event trail.
    /// The `NOT EXISTS` sibling guard defers reaping when #870's correction
    /// loop has launched a retry via `create_task`.
    ///
    /// Any new writers of `callback_delivered_without_pr_url` must respect
    /// the groom-class filter. SOLE WRITER: this method is the only site
    /// that writes `callback_delivered_without_pr_url` to `tasks.result`.
    /// mika#1705: ingest finished claude-pilot transcript JSONL files into the
    /// `pilot_transcripts` table, then delete each imported file.
    ///
    /// Runs on the periodic DB scan. Files live in a single shared directory
    /// (`{home}/data/pilot-transcripts/<callback-task-id>.jsonl`); because
    /// [`AsyncDatabase::get_task`] is agent-scoped, each file resolves to
    /// exactly one engine (the dispatching agent's), so N engines scanning the
    /// same directory never double-import — non-owners see `None` and skip.
    ///
    /// Race safety (mika#1705 Risk 5): a file is only imported once its owning
    /// callback task has left `in_progress`/`pending` — by then the pilot
    /// subprocess has exited (dispatch-lib's EXIT trap completes the task only
    /// after `claude-pilot` returns), so the JSONL is fully written. Import is
    /// transactional (all rows or none) and the file is deleted only after a
    /// successful commit; a pre-existing row count short-circuits to delete,
    /// giving idempotency when a prior commit succeeded but the unlink failed.
    async fn ingest_pilot_transcripts(&self) {
        if !crate::skills::executor::pilot_transcripts_enabled() {
            return;
        }
        let dir = self
            .dispatcher
            .settings
            .home_dir
            .join("data")
            .join("pilot-transcripts");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // Dir absent = nothing dispatched with capture yet. Not an error.
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(task_id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };

            // Agent-scoped lookup routes each file to exactly one engine.
            let task = match self.db.get_task(&task_id).await {
                Ok(Some(t)) => t,
                Ok(None) => continue, // not this agent's task — leave for owner
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "mika#1705: task lookup failed; skipping transcript file");
                    continue;
                }
            };

            // Only import once the pilot subprocess has exited (task no longer
            // pending/in_progress) so we never read a partially-written file.
            if matches!(
                task.status.as_str(),
                task_status::PENDING | task_status::IN_PROGRESS
            ) {
                continue;
            }

            // Idempotency: rows already present ⇒ a prior import committed but
            // the unlink failed. Just delete the file and move on.
            match self
                .db
                .count_pilot_transcripts_for_task(task_id.clone())
                .await
            {
                Ok(n) if n > 0 => {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(task_id = %task_id, error = %e, "mika#1705: failed to delete already-imported transcript file");
                    }
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "mika#1705: transcript count check failed; skipping");
                    continue;
                }
            }

            let contents = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "mika#1705: failed to read transcript file");
                    continue;
                }
            };

            let rows: Vec<crate::db::PilotTranscriptRow> = contents
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .map(|v| parse_pilot_transcript_line(&v))
                .collect();

            if rows.is_empty() {
                // Empty or all-unparseable file: delete so it doesn't linger.
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!(task_id = %task_id, error = %e, "mika#1705: failed to delete empty transcript file");
                }
                continue;
            }

            match self
                .db
                .insert_pilot_transcripts_batch(task_id.clone(), rows)
                .await
            {
                Ok(imported) => {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(task_id = %task_id, error = %e, "mika#1705: import committed but file delete failed (idempotent next tick)");
                    }
                    info!(task_id = %task_id, imported, "mika#1705: ingested pilot transcript");
                }
                Err(e) => {
                    // Leave the file in place — retried on the next scan.
                    warn!(task_id = %task_id, error = %e, "mika#1705: transcript import failed; will retry");
                }
            }
        }
    }

    /// mika#1705 AC6: delete `pilot_transcripts` rows older than the retention
    /// window (`MIKA_PILOT_TRANSCRIPT_RETENTION_DAYS`, default 90). Called at
    /// startup and once per [`PILOT_TRANSCRIPT_RETENTION_INTERVAL_TICKS`].
    async fn prune_old_pilot_transcripts(&self) {
        let days = std::env::var(PILOT_TRANSCRIPT_RETENTION_ENV)
            .ok()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|d| *d > 0)
            .unwrap_or(PILOT_TRANSCRIPT_RETENTION_DEFAULT_DAYS);
        let retention_secs = days * 24 * 60 * 60;
        match self.db.prune_old_pilot_transcripts(retention_secs).await {
            Ok(n) if n > 0 => info!(count = n, days, "mika#1705: pruned old pilot transcripts"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "mika#1705: failed to prune old pilot transcripts"),
        }
    }

    async fn reap_orphaned_parent_tasks(&self) {
        let candidates = match self
            .db
            .find_orphaned_parent_tasks(REAPER_GRACE_SECONDS)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "task_engine_reaper: failed to query orphaned parents");
                return;
            }
        };

        for parent in candidates {
            let trace_id = mika_common::trace::generate_trace_id();
            let system_session = format!("system-{}", parent.agent_id);

            // mika#1126 AC-1: snapshot ALL children at decision time for
            // post-incident diagnosis of what the reaper saw.
            let children = match self.db.get_reaper_child_snapshot(&parent.id).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        parent_id = %parent.id,
                        error = %e,
                        "task_engine_reaper: failed to snapshot children"
                    );
                    // Continue with the kill — snapshot failure is non-fatal.
                    // The reaper already decided to reap via the SQL query.
                    Vec::new()
                }
            };

            // mika#1126 AC-1: structured log of what the reaper evaluated
            info!(
                parent_id = %parent.id,
                parent_status = "in_progress",
                parent_source = "self_dev",
                callback_task_id = %parent.callback_task_id,
                children_count = children.len(),
                children = ?children,
                "task_engine_reaper.evaluated"
            );

            // mika#1126 AC-3: defense-in-depth — re-check child dispatch_class
            // at kill time. The SQL query should have excluded groom-class children,
            // but if the class was NULL at query time and populated since, this
            // guard catches the TOCTOU race (H2).
            if !children.is_empty() {
                let delivered_callback_children: Vec<_> = children
                    .iter()
                    .filter(|c| {
                        c.trigger_type == "callback"
                            && c.action_type == "resume_agent"
                            && c.status == "delivered"
                    })
                    .collect();

                if !delivered_callback_children.is_empty() {
                    let all_non_implement = delivered_callback_children
                        .iter()
                        .all(|c| c.dispatch_class.as_deref().unwrap_or("implement") != "implement");

                    if all_non_implement {
                        warn!(
                            parent_id = %parent.id,
                            children = ?children,
                            "task_engine_reaper: race detected — all delivered callback \
                             children are non-implement class at kill time; skipping \
                             reap (mika#1126 guard)"
                        );
                        continue;
                    }
                }
            }

            // SOLE WRITER: callback_delivered_without_pr_url
            // Use update_task_failed (guarded UPDATE with terminal-state check)
            // instead of raw update_task_status to avoid overwriting concurrent
            // terminal transitions. Returns false when the parent already left
            // in_progress (race with operator action or duplicate query rows).
            match self
                .db
                .update_task_failed(&parent.id, "callback_delivered_without_pr_url")
                .await
            {
                Ok(true) => {
                    // Transition succeeded — emit audit event
                    if let Err(e) = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_reaper",
                            &parent.id,
                            Some("in_progress"),
                            Some("failed"),
                            Some("callback_delivered_without_pr_url"),
                            Some(&trace_id),
                        )
                        .await
                    {
                        warn!(
                            parent_id = %parent.id,
                            error = %e,
                            "task_engine_reaper: failed to write audit event"
                        );
                    }

                    // F6: surface pre-existing leaks reaped from before deploy
                    let age_hours = compute_reaper_age_hours(&parent.created_at);
                    if age_hours > 24 {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            age_hours,
                            "task_engine_reaper: reaping pre-existing orphan \
                             (possible backfill from before reaper deployment)"
                        );
                    } else {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            "task_engine_reaper: transitioned orphaned parent to failed"
                        );
                    }
                }
                Ok(false) => {
                    // Parent already transitioned away from in_progress
                    // (concurrent operator action or duplicate query row) — skip.
                    debug!(
                        parent_id = %parent.id,
                        "task_engine_reaper: parent already in terminal state, skipping"
                    );
                }
                Err(e) => {
                    // F5: audit-event-on-error so operators catch silent-reaper-failure
                    let _ = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_reaper",
                            &parent.id,
                            Some("in_progress"),
                            None,
                            Some(&format!("reaper_db_error: {e}")),
                            Some(&trace_id),
                        )
                        .await;
                    warn!(
                        parent_id = %parent.id,
                        error = %e,
                        "task_engine_reaper: db error during transition"
                    );
                }
            }
        }
    }

    /// Auto-complete parent self_dev tasks whose callback delivered with a
    /// `pr_url` but were never transitioned by the silent agent turn (mika#1162).
    ///
    /// Success-side sibling to `reap_orphaned_parent_tasks`. Same scan cadence
    /// (every `DB_SCAN_INTERVAL_TICKS`), same agent/source/trigger_type and
    /// `dispatch_class='implement'` filters, same `REAPER_GRACE_SECONDS` grace
    /// window — but transitions matching parents to `completed` instead of
    /// `failed`. Mutually exclusive with the reaper on the `pr_url` predicate
    /// (reaper requires `IS NULL`, completer requires `IS NOT NULL`), so the
    /// two queries never select the same row.
    ///
    /// Layered with the inline path in `dispatcher::try_complete_parent_on_callback_success`:
    /// the inline path fires at delivery time and frees the slot fast; this
    /// periodic backstop catches crash-recovery cases (server died between
    /// callback delivery and the inline call) and pre-deploy wedges.
    ///
    /// SOLE WRITER: this method and the inline counterpart are the only sites
    /// that write the `parent_completed_from_callback` audit-event transition.
    async fn complete_parent_tasks_on_callback_success(&self) {
        let candidates = match self
            .db
            .find_completable_parent_tasks_on_pr_url(REAPER_GRACE_SECONDS)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    error = %e,
                    "task_engine_parent_completer: failed to query completable parents"
                );
                return;
            }
        };

        // Asymmetry with reaper (mika#1126 AC-3): the reaper performs a kill-time
        // re-fetch of all children and re-verifies dispatch_class because a
        // groom-class child masquerading as implement could falsely trigger
        // `failed`. The completer skips this re-check because the `pr_url IS NOT
        // NULL` predicate is an independent guard — groom-class callbacks never
        // emit `PR:` lines, so a parent with pr_url in metadata cannot be in the
        // class-race scenario mika#1126 protects against. Concurrent operator
        // races are caught by `update_task_completed`'s `status IN (...)` guard.
        for parent in candidates {
            let trace_id = mika_common::trace::generate_trace_id();
            let system_session = format!("system-{}", parent.agent_id);
            let reason = format!(
                "parent_completed_from_callback_backstop (pr_url: {})",
                parent.pr_url
            );

            match self
                .db
                .update_task_completed(&parent.id, Some(&reason))
                .await
            {
                Ok(true) => {
                    if let Err(e) = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_parent_completer",
                            &parent.id,
                            Some("in_progress"),
                            Some("completed"),
                            Some(&reason),
                            Some(&trace_id),
                        )
                        .await
                    {
                        warn!(
                            parent_id = %parent.id,
                            error = %e,
                            "task_engine_parent_completer: failed to write audit event"
                        );
                    }

                    // Surface pre-existing leaks reaped from before deploy
                    // (mirror reaper's F6 pattern).
                    let age_hours = compute_reaper_age_hours(&parent.created_at);
                    if age_hours > 24 {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            pr_url = %parent.pr_url,
                            age_hours,
                            "task_engine_parent_completer: auto-completed pre-existing wedged \
                             parent (possible backfill from before completer deployment)"
                        );
                    } else {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            pr_url = %parent.pr_url,
                            "task_engine_parent_completer: auto-completed parent task on \
                             callback success"
                        );
                    }
                }
                Ok(false) => {
                    // Parent already transitioned away from in_progress
                    // (race with inline path or operator action) — skip.
                    debug!(
                        parent_id = %parent.id,
                        "task_engine_parent_completer: parent already in terminal state, skipping"
                    );
                }
                Err(e) => {
                    // Audit-event-on-error so operators catch silent failures
                    // (mirror reaper's F5 pattern).
                    let _ = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_parent_completer",
                            &parent.id,
                            Some("in_progress"),
                            None,
                            Some(&format!("completer_db_error: {e}")),
                            Some(&trace_id),
                        )
                        .await;
                    warn!(
                        parent_id = %parent.id,
                        error = %e,
                        "task_engine_parent_completer: db error during transition"
                    );
                }
            }
        }
    }

    /// Reap parent self_dev **issue** tasks left `in_progress` with **zero**
    /// callback children, aged past the childless grace window (mika#1687).
    ///
    /// The deterministic backstop for silent pilot death: a parent that reaches
    /// `in_progress` but never records a callback child falls through all three
    /// existing deterministic mechanisms — the orphan reaper (#871) and
    /// parent-completer (mika#1162) both INNER-JOIN a delivered callback child,
    /// and the callback watchdog (#959) keys off the callback child's PID. This
    /// reaper's `NOT EXISTS` predicate is the exact complement of that JOIN, so
    /// it sees exactly the parents the others cannot.
    ///
    /// Its job is **fail-with-telemetry**, not re-drive: it transitions the
    /// parent to `failed` so the death is visible and terminal (freeing the
    /// dispatch slot + emitting a greppable signal). Re-driving the still-open
    /// ticket is mika#1824's job at the auto-pull/label layer (D3).
    ///
    /// SOLE WRITER: this method is the only site that writes the
    /// `stuck_in_progress_no_callback_child` reason to `tasks.result` and the
    /// only writer of `task_engine_childless_reaper` audit events. Reusing
    /// either string elsewhere breaks the operator/monitor discriminator (R4,
    /// D4) — keep this method the single writer.
    async fn reap_childless_stuck_parent_tasks(&self) {
        let grace_seconds = childless_parent_reaper_grace_secs();
        let candidates = match self
            .db
            .find_childless_stuck_parent_tasks(grace_seconds)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    error = %e,
                    "task_engine_childless_reaper: failed to query childless stuck parents"
                );
                return;
            }
        };

        for parent in candidates {
            let trace_id = mika_common::trace::generate_trace_id();
            let system_session = format!("system-{}", parent.agent_id);

            // Diagnostic parity with the #1126 reaper snapshot: confirm at
            // decision time that the parent truly has zero children (the
            // absence this reaper acts on). Snapshot failure is non-fatal — the
            // SQL query already decided via `NOT EXISTS`.
            let children = match self.db.get_reaper_child_snapshot(&parent.id).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        parent_id = %parent.id,
                        error = %e,
                        "task_engine_childless_reaper: failed to snapshot children"
                    );
                    Vec::new()
                }
            };
            info!(
                parent_id = %parent.id,
                agent_id = %parent.agent_id,
                parent_status = "in_progress",
                parent_source = "self_dev",
                children_count = children.len(),
                children = ?children,
                "task_engine_childless_reaper.evaluated"
            );

            // SOLE WRITER: stuck_in_progress_no_callback_child. Guarded UPDATE
            // (terminal-state check) — returns false when the parent already
            // left in_progress (operator/agent race), true on transition.
            match self
                .db
                .update_task_failed(&parent.id, "stuck_in_progress_no_callback_child")
                .await
            {
                Ok(true) => {
                    if let Err(e) = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_childless_reaper",
                            &parent.id,
                            Some("in_progress"),
                            Some("failed"),
                            Some("stuck_in_progress_no_callback_child"),
                            Some(&trace_id),
                        )
                        .await
                    {
                        warn!(
                            parent_id = %parent.id,
                            error = %e,
                            "task_engine_childless_reaper: failed to write audit event"
                        );
                    }

                    let age_minutes = compute_reaper_age_minutes(&parent.created_at);
                    info!(
                        parent_id = %parent.id,
                        agent_id = %parent.agent_id,
                        created_at = %parent.created_at,
                        age_minutes,
                        trace_id = %trace_id,
                        "task_engine_childless_reaper.reaped"
                    );
                }
                Ok(false) => {
                    // Parent already transitioned away from in_progress
                    // (operator/agent race or duplicate query row) — skip (R7).
                    debug!(
                        parent_id = %parent.id,
                        "task_engine_childless_reaper: parent already in terminal state, skipping"
                    );
                }
                Err(e) => {
                    // Audit-event-on-error so operators catch silent failures
                    // (mirror the orphan reaper's F5 pattern, R7).
                    let _ = self
                        .db
                        .log_audit_event(
                            &system_session,
                            "task_engine_childless_reaper",
                            &parent.id,
                            Some("in_progress"),
                            None,
                            Some(&format!("reaper_db_error: {e}")),
                            Some(&trace_id),
                        )
                        .await;
                    warn!(
                        parent_id = %parent.id,
                        error = %e,
                        "task_engine_childless_reaper: db error during transition"
                    );
                }
            }
        }
    }

    /// Compute the fire timestamp and push a task onto the heap (used in both
    /// startup recovery and periodic scan).
    #[allow(clippy::too_many_arguments)]
    fn enqueue_queued_task(
        &mut self,
        task_id: &str,
        trigger_type_str: &str,
        action_type_str: &str,
        cron_expr: Option<&str>,
        next_fire_at: Option<&str>,
        now: &str,
        metadata: Option<&str>,
    ) {
        if self.queued_ids.contains(task_id) {
            return;
        }
        // Extract timezone from metadata for timezone-aware cron evaluation
        let parsed_tz = extract_timezone_from_metadata(metadata)
            .and_then(|tz_str| parse_timezone(&tz_str).ok());

        let fire_at = if trigger_type_str == trigger_type::RECURRING {
            match cron_expr {
                Some(expr) => {
                    let result = if let Some(ref tz) = parsed_tz {
                        next_fire_from_cron_tz(expr, now, tz)
                    } else {
                        next_fire_from_cron(expr, now)
                    };
                    match result {
                        Ok(ts) => ts,
                        Err(e) => {
                            warn!(task_id, error = %e, "failed to compute cron next fire");
                            return;
                        }
                    }
                }
                None => {
                    warn!(task_id, "recurring task missing cron_expr");
                    return;
                }
            }
        } else {
            match next_fire_at {
                Some(ts) => ts.to_string(),
                None => {
                    warn!(task_id, "task missing next_fire_at");
                    return;
                }
            }
        };

        self.push_to_heap(QueuedTask {
            task_id: task_id.to_owned(),
            next_fire_at: fire_at,
            trigger_type: trigger_type_str.to_owned(),
            action_type: action_type_str.to_owned(),
            cron_expr: cron_expr.map(str::to_owned),
        });
    }

    fn push_to_heap(&mut self, task: QueuedTask) {
        self.queued_ids.insert(task.task_id.clone());
        self.queue.push(task);
    }

    fn pop_from_heap(&mut self) -> Option<QueuedTask> {
        let task = self.queue.pop()?;
        self.queued_ids.remove(&task.task_id);
        Some(task)
    }

    /// Mark the task `in_progress` in DB, then spawn a task to dispatch it.
    ///
    /// Dispatch runs in a spawned task so the engine lock is released immediately.
    /// Long-running dispatchers (run_skill) can hold the spawned task for up to
    /// 300s without blocking the tick loop.
    async fn fire_task(&mut self, queued: QueuedTask) {
        let task_id = queued.task_id.clone();

        // Atomically claim the task and record fired_at in one DB round-trip
        match self.db.claim_and_fire_task(&task_id).await {
            Ok(true) => {} // claimed successfully
            Ok(false) => {
                debug!(task_id = %task_id, "task no longer claimable (cancelled/completed/expired), skipping");
                return;
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "failed to claim task");
                return;
            }
        }

        let dispatcher = self.dispatcher.clone();
        let db = self.db.clone();
        let reenqueue_tx = self.reenqueue_tx.clone();
        let trigger_type_val = queued.trigger_type.clone();
        let action_type_val = queued.action_type.clone();
        let cron_expr = queued.cron_expr.clone();

        tokio::spawn(async move {
            let result = dispatcher.dispatch(&task_id).await;
            match result {
                Ok(()) => {
                    if trigger_type_val == trigger_type::RECURRING {
                        // Read timezone from task metadata for timezone-aware rescheduling
                        let parsed_tz = match db.get_task(&task_id).await {
                            Ok(Some(task)) => {
                                extract_timezone_from_metadata(task.metadata.as_deref())
                                    .and_then(|tz_str| parse_timezone(&tz_str).ok())
                            }
                            Ok(None) => None,
                            Err(e) => {
                                warn!(task_id = %task_id, error = %e, "failed to read task for timezone metadata, falling back to UTC");
                                None
                            }
                        };

                        // Recompute next fire time and re-enqueue
                        let now_str = crate::timestamp::now();
                        let next = match cron_expr
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("recurring task missing cron_expr"))
                            .and_then(|e| {
                                if let Some(ref tz) = parsed_tz {
                                    next_fire_from_cron_tz(e, &now_str, tz)
                                } else {
                                    next_fire_from_cron(e, &now_str)
                                }
                            }) {
                            Ok(ts) => ts,
                            Err(e) => {
                                warn!(task_id = %task_id, error = %e, "cannot reschedule recurring task, marking failed");
                                if let Err(db_err) =
                                    db.update_task_failed(&task_id, &e.to_string()).await
                                {
                                    warn!(task_id = %task_id, error = %db_err, "failed to mark recurring task as failed in DB");
                                }
                                return;
                            }
                        };

                        if let Err(e) = db.update_task_rescheduled(&task_id, &next).await {
                            warn!(task_id = %task_id, error = %e, "failed to reschedule recurring task after fire");
                        }

                        // Send back to engine for re-enqueue on next tick
                        let _ = reenqueue_tx
                            .send(QueuedTask {
                                task_id,
                                next_fire_at: next,
                                trigger_type: trigger_type_val,
                                action_type: action_type_val.clone(),
                                cron_expr,
                            })
                            .await;
                    } else if action_type_val != action_type::INJECT_CONTEXT {
                        // inject_context stays in_progress until the agent loop consumes it.
                        // All other one-shot actions are marked completed here.
                        match db.update_task_completed(&task_id, None).await {
                            Ok(false) => {
                                warn!(task_id = %task_id, "task already in terminal state, skipping complete");
                            }
                            Err(e) => {
                                warn!(task_id = %task_id, error = %e, "failed to mark task completed");
                            }
                            Ok(true) => {
                                dispatcher.check_and_dispatch_parent(&task_id).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    if matches!(e, super::dispatcher::DispatchError::AgentBusy(_)) {
                        // Check if the task has expired before re-queuing
                        let now = crate::timestamp::now();
                        let is_expired = match db.get_task(&task_id).await {
                            Ok(Some(t)) => {
                                t.timeout_at.as_deref().is_some_and(|ts| ts <= now.as_str())
                            }
                            _ => false,
                        };

                        if is_expired {
                            warn!(task_id = %task_id, "task timed out while waiting for agent, marking failed");
                            if let Err(db_err) = db
                                .update_task_failed(
                                    &task_id,
                                    "task timed out while waiting for agent",
                                )
                                .await
                            {
                                warn!(task_id = %task_id, error = %db_err, "failed to mark timed-out task as failed in DB");
                            }
                        } else {
                            // Agent is busy — reset task to pending and re-enqueue for retry
                            debug!(task_id = %task_id, "agent busy, re-queuing task for retry in 30s");
                            let retry_at =
                                crate::timestamp::now_plus(chrono::Duration::seconds(30));
                            if let Err(e) =
                                db.update_task_status(&task_id, task_status::PENDING).await
                            {
                                warn!(task_id = %task_id, error = %e, "failed to reset task status to pending for retry");
                            }
                            if let Err(e) = db.update_task_next_fire_at(&task_id, &retry_at).await {
                                warn!(task_id = %task_id, error = %e, "failed to update next_fire_at for retry");
                            }
                            let _ = reenqueue_tx
                                .send(QueuedTask {
                                    task_id,
                                    next_fire_at: retry_at,
                                    trigger_type: trigger_type_val,
                                    action_type: action_type_val,
                                    cron_expr,
                                })
                                .await;
                        }
                    } else {
                        let err_msg = e.to_string();
                        warn!(task_id = %task_id, error = %err_msg, "task dispatch failed");
                        if let Err(db_err) = db.update_task_failed(&task_id, &err_msg).await {
                            warn!(task_id = %task_id, error = %db_err, "failed to mark task as failed in DB");
                        }
                    }
                }
            }
        });
        // Engine lock released when fire_task() returns (immediately after spawn)
    }
}

/// Compute how many hours old a task is based on its `created_at` timestamp.
/// Returns 0 on parse failure (conservative — won't trigger the backfill log).
fn compute_reaper_age_hours(created_at: &str) -> i64 {
    crate::timestamp::parse(created_at)
        .map(|dt| {
            let now = chrono::Utc::now();
            (now - dt).num_hours()
        })
        .unwrap_or(0)
}

/// Compute how many minutes old a task is based on its `created_at` timestamp.
/// Returns 0 on parse failure (conservative — won't inflate the reaped-log age).
fn compute_reaper_age_minutes(created_at: &str) -> i64 {
    crate::timestamp::parse(created_at)
        .map(|dt| {
            let now = chrono::Utc::now();
            (now - dt).num_minutes()
        })
        .unwrap_or(0)
}

/// Pure parse of the childless-parent reaper grace window from an optional env
/// value (mika#1687, D5). Returns [`CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS`]
/// (1800) when the value is absent, empty, or unparseable/≤0 (WARN on invalid).
/// Split out from the env read so it is unit-testable without mutating process
/// environment (mirrors `parse_stuck_ready_threshold` in `auto_pull.rs`).
fn parse_childless_parent_reaper_grace(raw: Option<&str>) -> i64 {
    match raw {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<i64>() {
            Ok(secs) if secs > 0 => secs,
            _ => {
                warn!(
                    env = CHILDLESS_PARENT_REAPER_GRACE_ENV,
                    value = %v,
                    default = CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS,
                    "invalid childless-parent reaper grace value; falling back to default"
                );
                CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
            }
        },
        _ => CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS,
    }
}

/// Resolve the childless-parent reaper grace window (mika#1687, D5).
///
/// Reads `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` and delegates to
/// [`parse_childless_parent_reaper_grace`]. Same safe-fallback shape as the
/// watchdog's `effective_callback_watchdog_grace_period_secs()`. Env read once
/// per tick is negligible at the 60s DB-scan cadence.
fn childless_parent_reaper_grace_secs() -> i64 {
    parse_childless_parent_reaper_grace(
        std::env::var(CHILDLESS_PARENT_REAPER_GRACE_ENV)
            .ok()
            .as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewTask};
    use crate::messaging::MessageSender;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new_with_agent(db, "mika")
    }

    struct NoopSender;
    #[async_trait::async_trait]
    impl MessageSender for NoopSender {
        async fn send(&self, _text: &str) -> anyhow::Result<crate::messaging::SendOutcome> {
            Ok(crate::messaging::SendOutcome::Delivered)
        }
    }

    fn test_dispatcher(db: AsyncDatabase) -> Arc<TaskDispatcher> {
        let tmp = tempfile::tempdir().unwrap();
        let settings = mika_common::config::Settings::load(tmp.path()).unwrap();
        Arc::new(TaskDispatcher {
            db,
            llm: mika_common::llm::dummy_provider(),
            tools: Arc::new(crate::tools::default_tools()),
            skills: Arc::new(crate::skills::SkillRegistry::empty()),
            message_sender: Some(Arc::new(NoopSender)),
            home_dir: PathBuf::from("/tmp"),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            gateway_url: None,
            internal_token: None,
            github_app: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: false,
            settings,
            pr_reviews_posted: None,
        })
    }

    fn make_task(label: &str, next_fire_at: &str) -> NewTask {
        NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(next_fire_at.to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"text": "hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[tokio::test]
    async fn test_startup_recovery_empty_db() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db, dispatcher);
        engine.startup_recovery().await.unwrap();
        assert_eq!(engine.queue.len(), 0);
    }

    #[tokio::test]
    async fn test_enqueue_adds_to_heap_and_id_set() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db, dispatcher);

        let future_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
        let id = engine
            .enqueue(make_task("test reminder", &future_ts))
            .await
            .unwrap();

        assert!(!id.is_empty());
        assert_eq!(engine.queue.len(), 1);
        assert!(engine.queued_ids.contains(&id));
        assert_eq!(engine.queue.peek().unwrap().next_fire_at, future_ts);
    }

    #[tokio::test]
    async fn test_tick_fires_due_task() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let past_ts = crate::timestamp::now_minus(chrono::Duration::seconds(10));
        let id = engine
            .enqueue(make_task("past reminder", &past_ts))
            .await
            .unwrap();
        assert_eq!(engine.queue.len(), 1);

        engine.tick().await;
        assert_eq!(engine.queue.len(), 0);
        assert!(!engine.queued_ids.contains(&id));

        // Poll until completed (up to 5 seconds)
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let t = db.get_task(&id).await.unwrap().unwrap();
            if t.status == "completed" {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!(
                    "timed out waiting for task to complete; status={}",
                    t.status
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn test_tick_does_not_fire_future_task() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db, dispatcher);

        let future_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
        engine
            .enqueue(make_task("future reminder", &future_ts))
            .await
            .unwrap();
        engine.tick().await;

        assert_eq!(engine.queue.len(), 1);
    }

    #[tokio::test]
    async fn test_tick_skips_cancelled_task() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let past_ts = crate::timestamp::now_minus(chrono::Duration::seconds(10));
        let id = engine
            .enqueue(make_task("cancelled reminder", &past_ts))
            .await
            .unwrap();

        // Cancel in DB before tick fires it
        db.cancel_task(&id).await.unwrap();

        engine.tick().await;

        // Task should remain cancelled (not in_progress or completed)
        let t = db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(t.status, "cancelled");
    }

    #[tokio::test]
    async fn test_periodic_scan_picks_up_new_tasks() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // Create task directly in DB (bypassing engine.enqueue)
        let past_ts = crate::timestamp::now_minus(chrono::Duration::seconds(10));
        let id = db
            .create_task(make_task("direct db task", &past_ts))
            .await
            .unwrap();

        assert!(!engine.queued_ids.contains(&id));

        // Periodic scan should pick it up
        engine.scan_db_for_new_tasks().await;
        assert!(engine.queued_ids.contains(&id));

        // Second scan should not double-add
        engine.scan_db_for_new_tasks().await;
        assert_eq!(engine.queue.len(), 1);
    }

    #[tokio::test]
    async fn test_expire_timed_out_tasks() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // Create a callback task with timeout_at in the past
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long-running-job".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: Some(crate::timestamp::now_minus(chrono::Duration::seconds(60))), // expired 60s ago
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();

        engine.expire_timed_out_tasks().await;

        let t = db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(t.status, task_status::EXPIRED);
    }

    #[tokio::test]
    async fn test_expire_does_not_touch_active_tasks() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // Task with timeout_at in the future — should NOT be expired
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "still-running".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: Some(crate::timestamp::now_plus(chrono::Duration::seconds(3600))),
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(task).await.unwrap();

        engine.expire_timed_out_tasks().await;

        let t = db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(t.status, task_status::PENDING);
    }

    #[tokio::test]
    async fn test_expired_child_unblocks_parent() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        // Create parent task
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "parent".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::INVOKE_ORCHESTRATOR.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let parent_id = db.create_task(parent).await.unwrap();

        // Create two children: one completed, one expired
        let child1 = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "child-ok".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let c1_id = db.create_task(child1).await.unwrap();
        db.update_task_completed(&c1_id, Some("done"))
            .await
            .unwrap();

        let child2 = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "child-expired".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: Some(crate::timestamp::now_minus(chrono::Duration::seconds(60))),
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let c2_id = db.create_task(child2).await.unwrap();
        // Mark expired manually (in real flow, expire_timed_out_tasks does this)
        db.update_task_status(&c2_id, task_status::EXPIRED)
            .await
            .unwrap();

        // check_expired_siblings should detect that both children are terminal
        engine.check_expired_siblings().await;

        // Parent should have been claimed (dispatched) — status changed from pending
        // Note: dispatch will fail (no real agent), but the attempt proves the logic works.
        // Give spawned task a moment to run
        tokio::time::sleep(Duration::from_millis(100)).await;

        let parent_task = db.get_task(&parent_id).await.unwrap().unwrap();
        // Parent should be in_progress (claimed by dispatch) or failed (dispatch error)
        assert!(
            parent_task.status == "in_progress" || parent_task.status == "failed",
            "expected in_progress or failed, got: {}",
            parent_task.status
        );
    }

    #[tokio::test]
    async fn test_startup_recovery_marks_orphaned_in_progress_failed() {
        let db = test_db();

        let task_id = db
            .create_task(make_task(
                "orphan",
                &crate::timestamp::now_plus(chrono::Duration::seconds(3600)),
            ))
            .await
            .unwrap();

        db.update_task_status(&task_id, "in_progress")
            .await
            .unwrap();

        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);
        engine.startup_recovery().await.unwrap();

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "failed");
    }

    /// Create a callback task that is already completed (as if `mika ask --task-id` ran).
    fn make_callback_task(label: &str) -> NewTask {
        NewTask {
            agent_id: "mika".to_string(),
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
            action_config: r#"{"trigger":"callback"}"#.to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[tokio::test]
    async fn test_cli_mode_skips_callback_dispatch() {
        let db = test_db();
        // Create dispatcher with cli_mode: true
        let tmp = tempfile::tempdir().unwrap();
        let settings = mika_common::config::Settings::load(tmp.path()).unwrap();
        let dispatcher = Arc::new(TaskDispatcher {
            db: db.clone(),
            llm: mika_common::llm::dummy_provider(),
            tools: Arc::new(crate::tools::default_tools()),
            skills: Arc::new(crate::skills::SkillRegistry::empty()),
            message_sender: Some(Arc::new(NoopSender)),
            home_dir: PathBuf::from("/tmp"),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            gateway_url: None,
            internal_token: None,
            github_app: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: true,
            settings,
            pr_reviews_posted: None,
        });
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        // Insert a completed callback task directly in DB
        let task_id = db
            .create_task(make_callback_task("long_running:test_tool"))
            .await
            .unwrap();
        db.update_task_completed(&task_id, Some("test result"))
            .await
            .unwrap();

        // Verify the task is completed
        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "completed");

        // Run enough ticks to trigger the DB scan (DB_SCAN_INTERVAL_TICKS = 60)
        for _ in 0..=DB_SCAN_INTERVAL_TICKS {
            engine.tick().await;
        }

        // In CLI mode, dispatch_undelivered_callbacks is skipped.
        // The task should still be in 'completed' status, NOT 'delivered'.
        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(
            task.status, "completed",
            "cli_mode should prevent engine from dispatching callbacks"
        );
    }

    // -- complete_parent_tasks_on_callback_success tests (mika#1162) --

    /// Helper: seed a `self_dev` parent in `in_progress` with `pr_url` metadata.
    /// Adds a `delivered` implement-class callback child whose `updated_at`
    /// is backdated past the reaper grace window.
    async fn seed_completable_parent(db: &AsyncDatabase, pr_url: &str) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement mika#1162".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let parent_id = db.create_task(parent).await.unwrap();
        db.update_task_status(&parent_id, "in_progress")
            .await
            .unwrap();
        let meta = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
        db.update_task_metadata(&parent_id, &meta).await.unwrap();

        let child = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let child_id = db.create_task(child).await.unwrap();
        db.update_task_completed(&child_id, Some("done"))
            .await
            .unwrap();
        db.mark_task_delivered(&child_id).await.unwrap();
        // Backdate the delivered child past the grace window
        backdate_task(db, &child_id).await;
        (parent_id, child_id)
    }

    /// Test-only helper: shove `updated_at` back by 700s to push a task past
    /// `REAPER_GRACE_SECONDS`. Uses `with_db` to drop into the underlying
    /// connection — there is no public AsyncDatabase method for time travel.
    async fn backdate_task(db: &AsyncDatabase, task_id: &str) {
        let id = task_id.to_string();
        db.with_db(move |inner| {
            inner.conn.execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_complete_parent_tasks_on_callback_success_happy_path() {
        let db = test_db();
        let pr_url = "https://github.com/senara-solutions/mika/pull/1234";
        let (parent_id, _child_id) = seed_completable_parent(&db, pr_url).await;

        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);
        engine.complete_parent_tasks_on_callback_success().await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "completed");
        let result = parent.result.unwrap();
        assert!(
            result.contains("parent_completed_from_callback_backstop"),
            "result must carry the backstop marker (distinct from inline path), got: {result}"
        );
        assert!(
            result.contains(pr_url),
            "result must embed the pr_url for audit traceability"
        );

        // R3 — audit event must be written. Same tool_name as the inline path
        // so consumers grep one name and see both call sites.
        let pid = parent_id.clone();
        let events = db
            .with_db(move |inner| {
                inner.list_audit_events_paginated(
                    "mika",
                    Some("task_engine_parent_completer"),
                    Some(&pid),
                    10,
                    0,
                )
            })
            .await
            .unwrap();
        assert_eq!(
            events.len(),
            1,
            "periodic backstop must log one audit event"
        );
        let event = &events[0];
        assert_eq!(event.before_value.as_deref(), Some("in_progress"));
        assert_eq!(event.after_value.as_deref(), Some("completed"));
        let reasoning = event.reasoning.as_deref().unwrap();
        assert!(reasoning.contains("parent_completed_from_callback_backstop"));
        assert!(reasoning.contains(pr_url));
    }

    #[tokio::test]
    async fn test_complete_parent_tasks_on_callback_success_idempotent_race_with_inline() {
        // If the inline path already completed the parent, the periodic backstop
        // is a no-op — the WHERE clause on `update_task_completed` guards it.
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, _child_id) = seed_completable_parent(&db, pr_url).await;

        // Simulate the inline path having already completed the parent.
        db.update_task_completed(&parent_id, Some("inline_path_won"))
            .await
            .unwrap();

        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);
        engine.complete_parent_tasks_on_callback_success().await;

        let parent = db.get_task_unscoped(&parent_id).await.unwrap().unwrap();
        assert_eq!(parent.status, "completed");
        // The inline path's reason must not be overwritten by the periodic
        // backstop (note: this race is also blocked by the query filter
        // `parent.status = 'in_progress'` — the WHERE clause is a second
        // line of defense).
        assert_eq!(parent.result.as_deref(), Some("inline_path_won"));
    }

    #[tokio::test]
    async fn test_reaper_and_completer_orthogonal_on_pr_url() {
        // Seed two parents: one with pr_url (completer territory), one without
        // (reaper territory). After running both methods in the same tick, each
        // handles its candidate without cross-contamination.
        let db = test_db();
        let pr_url = "https://github.com/x/y/pull/42";
        let (completer_parent, _completer_child) = seed_completable_parent(&db, pr_url).await;

        // Build a reaper candidate: same shape but no pr_url on the parent
        // metadata. Manual setup since the helper always sets pr_url.
        let parent_b = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Implement #other".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let reaper_parent = db.create_task(parent_b).await.unwrap();
        db.update_task_status(&reaper_parent, "in_progress")
            .await
            .unwrap();

        let child_b = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(reaper_parent.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        };
        let child_b_id = db.create_task(child_b).await.unwrap();
        db.update_task_completed(&child_b_id, Some("done"))
            .await
            .unwrap();
        db.mark_task_delivered(&child_b_id).await.unwrap();
        backdate_task(&db, &child_b_id).await;

        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        // Run both methods in the same tick (matching the production order).
        engine.reap_orphaned_parent_tasks().await;
        engine.complete_parent_tasks_on_callback_success().await;

        let p_completer = db
            .get_task_unscoped(&completer_parent)
            .await
            .unwrap()
            .unwrap();
        let p_reaper = db.get_task_unscoped(&reaper_parent).await.unwrap().unwrap();
        assert_eq!(
            p_completer.status, "completed",
            "parent with pr_url goes to completer"
        );
        assert_eq!(
            p_reaper.status, "failed",
            "parent without pr_url goes to reaper"
        );
    }

    // -- promote_pending_deferred_if_idle per-class iteration tests (mika#1175) --

    /// Seed a parent manual task + a `:deferred` callback wrapper child of the
    /// given `dispatch_class`. Returns `(parent_id, wrapper_id)`.
    async fn seed_deferred_wrapper(
        db: &AsyncDatabase,
        parent_label: &str,
        dispatch_class: Option<&str>,
    ) -> (String, String) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: parent_label.to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: dispatch_class.map(str::to_string),
        };
        let parent_id = db.create_task(parent).await.unwrap();

        let wrapper = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id.clone()),
            depth: 1,
            label: "long_running:run_claude_pilot:deferred".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: dispatch_class.map(str::to_string),
        };
        let wrapper_id = db.create_task(wrapper).await.unwrap();
        (parent_id, wrapper_id)
    }

    /// Seed a parent manual task + an active (pending, non-deferred) callback
    /// child of the given `dispatch_class`. Used to simulate a busy slot.
    async fn seed_active_non_deferred(
        db: &AsyncDatabase,
        parent_label: &str,
        dispatch_class: Option<&str>,
    ) {
        let parent = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: parent_label.to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: dispatch_class.map(str::to_string),
        };
        let parent_id = db.create_task(parent).await.unwrap();

        let child = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_id),
            depth: 1,
            label: "long_running:run_claude_pilot".to_string(),
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
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: dispatch_class.map(str::to_string),
        };
        db.create_task(child).await.unwrap();
    }

    /// R1: two cross-class deferred wrappers, both class slots idle → both
    /// promote in the same backstop tick.
    #[tokio::test]
    async fn test_promote_pending_deferred_if_idle_iterates_per_class() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        let (_, w_impl) = seed_deferred_wrapper(&db, "p_impl", Some("implement")).await;
        let (_, w_groom) = seed_deferred_wrapper(&db, "p_groom", Some("groom")).await;

        engine.promote_pending_deferred_if_idle().await;

        let t_impl = db.get_task(&w_impl).await.unwrap().unwrap();
        let t_groom = db.get_task(&w_groom).await.unwrap().unwrap();
        assert_eq!(
            t_impl.status, "completed",
            "implement wrapper must promote on a single tick when both classes idle"
        );
        assert_eq!(
            t_groom.status, "completed",
            "groom wrapper must promote in the same tick — per-class iteration \
             prevents the mika#1175 cross-class halving"
        );
    }

    /// R3: cross-class deferred wrappers, implement slot busy, groom slot idle
    /// → only the groom wrapper promotes.
    #[tokio::test]
    async fn test_promote_pending_deferred_if_idle_skips_busy_class() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        let (_, w_impl) = seed_deferred_wrapper(&db, "p_impl", Some("implement")).await;
        let (_, w_groom) = seed_deferred_wrapper(&db, "p_groom", Some("groom")).await;
        seed_active_non_deferred(&db, "p_busy_impl", Some("implement")).await;

        engine.promote_pending_deferred_if_idle().await;

        let t_impl = db.get_task(&w_impl).await.unwrap().unwrap();
        let t_groom = db.get_task(&w_groom).await.unwrap().unwrap();
        assert_eq!(
            t_impl.status, "pending",
            "implement wrapper must NOT promote when implement slot is busy"
        );
        assert_eq!(
            t_groom.status, "completed",
            "groom wrapper must promote — its class slot is independent of \
             the implement slot (mika#1175 per-class gate)"
        );
    }

    /// R2: with two implement deferred wrappers pending and the implement slot
    /// idle, exactly one wrapper promotes per backstop tick. Pins the
    /// at-most-one-per-class-per-tick invariant at the engine seam (the DB
    /// primitive's `LIMIT 1` is verified separately by
    /// `test_promote_next_deferred_callback_for_class_filters_by_class`).
    #[tokio::test]
    async fn test_promote_pending_deferred_if_idle_single_class_one_per_tick() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        let (_, w_impl_1) = seed_deferred_wrapper(&db, "p_impl_1", Some("implement")).await;
        let (_, w_impl_2) = seed_deferred_wrapper(&db, "p_impl_2", Some("implement")).await;

        engine.promote_pending_deferred_if_idle().await;

        // Exactly one wrapper must transition. Per-class tick budget is one
        // promotion; FIFO at second-resolution timestamps is best-effort
        // (correctness review C-01 — same-second wrappers tie-break by rowid),
        // so we assert "one and only one" rather than locking the specific row.
        let t1 = db.get_task(&w_impl_1).await.unwrap().unwrap();
        let t2 = db.get_task(&w_impl_2).await.unwrap().unwrap();
        let promoted = [&t1, &t2]
            .iter()
            .filter(|t| t.status == "completed")
            .count();
        let pending = [&t1, &t2].iter().filter(|t| t.status == "pending").count();
        assert_eq!(
            promoted, 1,
            "exactly one same-class wrapper must promote per tick (mika#1175 R2); \
             got promoted={promoted} pending={pending} t1={:?} t2={:?}",
            t1.status, t2.status
        );
        assert_eq!(pending, 1, "the other wrapper must stay pending");

        // A second tick promotes the remaining wrapper — confirms the loop
        // is not somehow stuck after the first promotion.
        engine.promote_pending_deferred_if_idle().await;
        let t1 = db.get_task(&w_impl_1).await.unwrap().unwrap();
        let t2 = db.get_task(&w_impl_2).await.unwrap().unwrap();
        assert_eq!(t1.status, "completed");
        assert_eq!(t2.status, "completed");
    }

    /// R4: with one implement + one groom deferred wrapper pending and BOTH
    /// class slots occupied, no wrapper promotes. Exercises the double-`continue`
    /// path through the per-class loop.
    #[tokio::test]
    async fn test_promote_pending_deferred_if_idle_both_classes_busy() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let engine = TaskEngine::new(db.clone(), dispatcher);

        let (_, w_impl) = seed_deferred_wrapper(&db, "p_impl", Some("implement")).await;
        let (_, w_groom) = seed_deferred_wrapper(&db, "p_groom", Some("groom")).await;
        seed_active_non_deferred(&db, "p_busy_impl", Some("implement")).await;
        seed_active_non_deferred(&db, "p_busy_groom", Some("groom")).await;

        engine.promote_pending_deferred_if_idle().await;

        assert_eq!(
            db.get_task(&w_impl).await.unwrap().unwrap().status,
            "pending",
            "implement wrapper must NOT promote when implement slot is busy (mika#1175 R4)"
        );
        assert_eq!(
            db.get_task(&w_groom).await.unwrap().unwrap().status,
            "pending",
            "groom wrapper must NOT promote when groom slot is busy (mika#1175 R4)"
        );
    }

    /// Drift detector for the `DISPATCH_CLASSES` slice. Every value returned by
    /// `derive_dispatch_class` for the currently-known skills must be in
    /// `DISPATCH_CLASSES`. If a new class is added to `derive_dispatch_class`
    /// (e.g., a third skill maps to a new class), this test surfaces the gap
    /// before the periodic backstop silently loses promotion for that class.
    ///
    /// COUPLED PAIR (mika#1175): the probe list below is hand-maintained. When
    /// adding a new arm to `derive_dispatch_class`, also add a representative
    /// skill input here. The cross-pointer at the match site in
    /// `skills/executor.rs` names this test as the required co-update.
    #[test]
    fn test_dispatch_classes_universe_matches_derive_fn() {
        use crate::skills::executor::derive_dispatch_class;

        for skill in [
            Some("dev-groom"),
            Some("dev-pilot"),
            Some("deploy_mika"),
            None,
        ] {
            let class = derive_dispatch_class(skill);
            assert!(
                DISPATCH_CLASSES.contains(&class),
                "derive_dispatch_class({skill:?}) = {class:?} not in DISPATCH_CLASSES \
                 = {DISPATCH_CLASSES:?}. If a new dispatch_class was added, update \
                 DISPATCH_CLASSES in engine.rs to include it (mika#1175)."
            );
        }
    }

    // -- childless-parent reaper grace reader tests (mika#1687, D5) --

    #[test]
    fn test_childless_grace_default_when_absent() {
        assert_eq!(
            parse_childless_parent_reaper_grace(None),
            CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
        );
    }

    #[test]
    fn test_childless_grace_default_when_empty() {
        assert_eq!(
            parse_childless_parent_reaper_grace(Some("   ")),
            CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
        );
    }

    #[test]
    fn test_childless_grace_valid_override() {
        assert_eq!(parse_childless_parent_reaper_grace(Some("3600")), 3600);
        // Surrounding whitespace is tolerated (trimmed before parse).
        assert_eq!(parse_childless_parent_reaper_grace(Some(" 900 ")), 900);
    }

    #[test]
    fn test_childless_grace_default_when_invalid() {
        assert_eq!(
            parse_childless_parent_reaper_grace(Some("not-a-number")),
            CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
        );
    }

    #[test]
    fn test_childless_grace_default_when_non_positive() {
        // Zero and negative are invalid (a legitimately-dispatching parent is
        // never childless for a non-positive window) → fall back to default.
        assert_eq!(
            parse_childless_parent_reaper_grace(Some("0")),
            CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
        );
        assert_eq!(
            parse_childless_parent_reaper_grace(Some("-300")),
            CHILDLESS_PARENT_REAPER_GRACE_DEFAULT_SECS
        );
    }
}
