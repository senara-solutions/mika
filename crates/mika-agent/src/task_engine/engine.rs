use anyhow::Result;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;

use super::cron::next_fire_from_cron;
use super::dispatcher::TaskDispatcher;
use super::queue::QueuedTask;

/// Maximum tasks fired per tick to prevent overrun when many tasks are overdue.
const MAX_PER_TICK: usize = 10;

/// How many ticks between periodic DB scans for tasks created outside the engine.
const DB_SCAN_INTERVAL_TICKS: u64 = 60;

/// Message sent from a dispatched task back to the engine to re-enqueue a
/// recurring task after it completes.
struct ReEnqueue {
    task_id: String,
    next_fire_at: i64,
    trigger_type: String,
    cron_expr: Option<String>,
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
    reenqueue_tx: mpsc::Sender<ReEnqueue>,
    reenqueue_rx: mpsc::Receiver<ReEnqueue>,
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
    pub async fn startup_recovery(&mut self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        // 1. Expire timed-out tasks
        match self.db.mark_tasks_expired(now).await {
            Ok(n) if n > 0 => info!(count = n, "expired timed-out tasks on startup"),
            Ok(_) => {}
            Err(e) => warn!(error = %e, "failed to expire timed-out tasks"),
        }

        // 2. Recover in_progress tasks (process couldn't have survived container restart)
        let in_progress = self
            .db
            .get_tasks_by_status(vec!["in_progress".to_string()])
            .await
            .unwrap_or_default();

        for task in in_progress {
            if task.process_id.is_some_and(process_is_alive) {
                debug!(task_id = %task.id, "in_progress task process still alive, skipping recovery");
                continue;
            }
            debug!(task_id = %task.id, "marking orphaned in_progress task as failed");
            if let Err(e) = self.db.update_task_status(&task.id, "failed").await {
                warn!(task_id = %task.id, error = %e, "failed to mark task as failed during recovery");
            }
        }

        // 3. Load schedulable tasks into BinaryHeap
        let schedulable = self.db.get_schedulable_tasks().await.unwrap_or_default();
        let count = schedulable.len();

        for task in schedulable {
            self.enqueue_queued_task(&task.id, &task.trigger_type, task.cron_expr.as_deref(), task.next_fire_at, now);
        }

        info!(
            loaded = count,
            queue_len = self.queue.len(),
            "task engine startup recovery complete"
        );
        Ok(())
    }

    /// Insert a new task into the DB and enqueue it in the BinaryHeap.
    ///
    /// Returns the new task's UUID string ID.
    pub async fn enqueue(&mut self, task: NewTask) -> Result<String> {
        let next_fire_at = task.next_fire_at;
        let trigger_type = task.trigger_type.clone();
        let cron_expr = task.cron_expr.as_deref().map(str::to_owned);

        let id = self.db.create_task(task).await?;

        if let Some(fire_at) = next_fire_at {
            self.push_to_heap(QueuedTask {
                task_id: id.clone(),
                next_fire_at: fire_at,
                trigger_type,
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
        while let Ok(re) = self.reenqueue_rx.try_recv() {
            self.push_to_heap(QueuedTask {
                task_id: re.task_id,
                next_fire_at: re.next_fire_at,
                trigger_type: re.trigger_type,
                cron_expr: re.cron_expr,
            });
        }

        // Periodic DB scan: pick up tasks created outside the engine (e.g. by tools)
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS) {
            self.scan_db_for_new_tasks().await;
        }

        let now = chrono::Utc::now().timestamp();
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

    /// Scan DB for schedulable tasks not already in the heap.
    /// Called every `DB_SCAN_INTERVAL_TICKS` seconds as a safety net.
    async fn scan_db_for_new_tasks(&mut self) {
        let now = chrono::Utc::now().timestamp();
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
            self.enqueue_queued_task(&task.id, &task.trigger_type, task.cron_expr.as_deref(), task.next_fire_at, now);
            added += 1;
        }
        if added > 0 {
            debug!(added, "periodic scan added new tasks to engine queue");
        }
    }

    /// Compute the fire timestamp and push a task onto the heap (used in both
    /// startup recovery and periodic scan).
    fn enqueue_queued_task(
        &mut self,
        task_id: &str,
        trigger_type: &str,
        cron_expr: Option<&str>,
        next_fire_at: Option<i64>,
        now: i64,
    ) {
        if self.queued_ids.contains(task_id) {
            return;
        }
        let fire_at = if trigger_type == "recurring" {
            match cron_expr {
                Some(expr) => match next_fire_from_cron(expr, now) {
                    Ok(ts) => ts,
                    Err(e) => {
                        warn!(task_id, error = %e, "failed to compute cron next fire");
                        return;
                    }
                },
                None => {
                    warn!(task_id, "recurring task missing cron_expr");
                    return;
                }
            }
        } else {
            match next_fire_at {
                Some(ts) => ts,
                None => {
                    warn!(task_id, "task missing next_fire_at");
                    return;
                }
            }
        };

        self.push_to_heap(QueuedTask {
            task_id: task_id.to_owned(),
            next_fire_at: fire_at,
            trigger_type: trigger_type.to_owned(),
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
    /// Long-running dispatchers (resume_agent, run_skill) can hold the spawned
    /// task for up to 300s without blocking the tick loop.
    async fn fire_task(&mut self, queued: QueuedTask) {
        let task_id = queued.task_id.clone();

        // Pre-check: skip if task was cancelled or already completed while queued
        match self.db.get_task(&task_id).await {
            Ok(Some(ref t)) if t.status == "cancelled" || t.status == "completed" => {
                debug!(task_id = %task_id, status = %t.status, "skipping non-pending task");
                return;
            }
            Ok(None) => {
                debug!(task_id = %task_id, "task not found, skipping");
                return;
            }
            _ => {}
        }

        if let Err(e) = self.db.update_task_status(&task_id, "in_progress").await {
            warn!(task_id = %task_id, error = %e, "failed to mark task in_progress");
            return;
        }
        if let Err(e) = self.db.set_task_fired(&task_id).await {
            warn!(task_id = %task_id, error = %e, "failed to set task fired_at");
        }

        let dispatcher = self.dispatcher.clone();
        let db = self.db.clone();
        let reenqueue_tx = self.reenqueue_tx.clone();
        let trigger_type = queued.trigger_type.clone();
        let cron_expr = queued.cron_expr.clone();

        tokio::spawn(async move {
            let result = dispatcher.dispatch(&task_id).await;
            match result {
                Ok(()) => {
                    if trigger_type == "recurring" {
                        // Recompute next fire time and re-enqueue
                        let next = cron_expr
                            .as_deref()
                            .and_then(|e| {
                                next_fire_from_cron(e, chrono::Utc::now().timestamp()).ok()
                            })
                            .unwrap_or(i64::MAX);

                        if let Err(e) = db.update_task_next_fire_at(&task_id, next).await {
                            warn!(task_id = %task_id, error = %e, "failed to update next_fire_at after recurring fire");
                        }
                        if let Err(e) = db.update_task_status(&task_id, "recurring_active").await {
                            warn!(task_id = %task_id, error = %e, "failed to set recurring_active status");
                        }

                        // Send back to engine for re-enqueue on next tick
                        let _ = reenqueue_tx
                            .send(ReEnqueue {
                                task_id,
                                next_fire_at: next,
                                trigger_type,
                                cron_expr,
                            })
                            .await;
                    } else if trigger_type != "inject_context" {
                        // inject_context stays in_progress until the agent loop consumes it.
                        // All other one-shot actions are marked completed here.
                        if let Err(e) = db.update_task_completed(&task_id, None).await {
                            warn!(task_id = %task_id, error = %e, "failed to mark task completed");
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "task dispatch failed");
                    if let Err(db_err) = db.update_task_completed(&task_id, Some(&e.to_string())).await {
                        warn!(task_id = %task_id, error = %db_err, "failed to mark task failed");
                    }
                }
            }
        });
        // Engine lock released when fire_task() returns (immediately after spawn)
    }
}

/// Check if a process is still alive by probing `/proc/{pid}` on Linux.
///
/// Returns `false` on any OS or permission error (safe default: treat as dead).
fn process_is_alive(pid: i64) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{}", pid)).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewTask};
    use crate::messaging::MessageSender;
    use mika_common::claude::ClaudeClient;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new_with_agent(db, "main")
    }

    struct NoopSender;
    #[async_trait::async_trait]
    impl MessageSender for NoopSender {
        async fn send(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn test_dispatcher(db: AsyncDatabase) -> Arc<TaskDispatcher> {
        let claude = ClaudeClient::new(
            Some("sk-test".to_string()),
            "claude-sonnet-4-6".to_string(),
            8192,
        )
        .unwrap();
        Arc::new(TaskDispatcher {
            db,
            claude,
            tools: Arc::new(crate::tools::default_tools()),
            skills: Arc::new(crate::skills::SkillRegistry::empty()),
            message_sender: Some(Arc::new(NoopSender)),
            home_dir: PathBuf::from("/tmp"),
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
        })
    }

    fn make_task(label: &str, next_fire_at: i64) -> NewTask {
        NewTask {
            agent_id: "main".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(next_fire_at),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"text": "hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
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

        let future_ts = chrono::Utc::now().timestamp() + 3600;
        let id = engine.enqueue(make_task("test reminder", future_ts)).await.unwrap();

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

        let past_ts = chrono::Utc::now().timestamp() - 10;
        let id = engine.enqueue(make_task("past reminder", past_ts)).await.unwrap();
        assert_eq!(engine.queue.len(), 1);

        engine.tick().await;
        assert_eq!(engine.queue.len(), 0);
        assert!(!engine.queued_ids.contains(&id));

        // Give spawned task time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        let t = db.get_task(&id).await.unwrap().unwrap();
        assert!(
            matches!(t.status.as_str(), "in_progress" | "completed"),
            "expected in_progress or completed, got {}",
            t.status
        );
    }

    #[tokio::test]
    async fn test_tick_does_not_fire_future_task() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db, dispatcher);

        let future_ts = chrono::Utc::now().timestamp() + 3600;
        engine.enqueue(make_task("future reminder", future_ts)).await.unwrap();
        engine.tick().await;

        assert_eq!(engine.queue.len(), 1);
    }

    #[tokio::test]
    async fn test_tick_skips_cancelled_task() {
        let db = test_db();
        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);

        let past_ts = chrono::Utc::now().timestamp() - 10;
        let id = engine.enqueue(make_task("cancelled reminder", past_ts)).await.unwrap();

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
        let past_ts = chrono::Utc::now().timestamp() - 10;
        let id = db.create_task(make_task("direct db task", past_ts)).await.unwrap();

        assert!(!engine.queued_ids.contains(&id));

        // Periodic scan should pick it up
        engine.scan_db_for_new_tasks().await;
        assert!(engine.queued_ids.contains(&id));

        // Second scan should not double-add
        engine.scan_db_for_new_tasks().await;
        assert_eq!(engine.queue.len(), 1);
    }

    #[tokio::test]
    async fn test_startup_recovery_marks_orphaned_in_progress_failed() {
        let db = test_db();

        let task_id = db
            .create_task(make_task("orphan", chrono::Utc::now().timestamp() + 3600))
            .await
            .unwrap();

        db.update_task_status(&task_id, "in_progress").await.unwrap();

        let dispatcher = test_dispatcher(db.clone());
        let mut engine = TaskEngine::new(db.clone(), dispatcher);
        engine.startup_recovery().await.unwrap();

        let task = db.get_task(&task_id).await.unwrap().unwrap();
        assert_eq!(task.status, "failed");
    }

    #[test]
    fn test_process_is_alive_current_process() {
        #[cfg(target_os = "linux")]
        {
            let pid = std::process::id() as i64;
            assert!(process_is_alive(pid));
        }
    }

    #[test]
    fn test_process_is_alive_invalid_pid() {
        assert!(!process_is_alive(i64::MAX));
    }
}
