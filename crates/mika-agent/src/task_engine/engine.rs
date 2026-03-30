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
use super::queue::QueuedTask;
use super::types::{action_type, task_status, trigger_type};

/// Maximum tasks fired per tick to prevent overrun when many tasks are overdue.
const MAX_PER_TICK: usize = 10;

/// How many ticks between periodic DB scans for tasks created outside the engine.
const DB_SCAN_INTERVAL_TICKS: u64 = 60;

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
        }
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
            // Manual (work item) tasks represent human work — don't invalidate on restart
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
    async fn tick(&mut self) {
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
            self.scan_db_for_new_tasks().await;
            // In CLI mode, the TUI's poll_callback_tasks() handles callback delivery
            // (session-scoped, atomic claim). Skip engine dispatch to prevent a race
            // where the engine steals callbacks and processes them in a context-free
            // silent turn. See #264.
            if !self.dispatcher.cli_mode {
                self.dispatch_undelivered_callbacks().await;
            }
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
                    // Use std::process::Command to send SIGTERM without adding a libc dependency
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .output();
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
            chrono::Duration::minutes(crate::agent::STALE_FAILED_CALLBACK_MINUTES);
        let mut stale_skipped: usize = 0;

        for task in tasks {
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
                            _ => None,
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
        async fn send(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
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
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: false,
            settings,
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
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            cli_mode: true,
            settings,
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
}
