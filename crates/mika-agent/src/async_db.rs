use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use tokio::sync::oneshot;

use crate::db::{
    AgentRow, AgentWithStats, AuditEvent, BackgroundTaskCounts, Commitment, CoreMemoryEntry,
    Database, Event, FailedSend, NewTask, Person, Preference, RecordOutcome, SearchResult,
    ServedContent, Session, SessionMessage, SessionWithStats, SkillOverride, Task, TaskFilters,
    TaskHealthSummary, TaskMessage, TaskSessionRow, TeamRow, TeamRunFilters, TeamRunRow,
    TeamRunSummary, TeamWorkspaceEntry, TimelineFilters, TimelineRow,
};
use crate::server::tasks_stream::{TaskEventFrame, TaskEventsChannel};

type DbClosure = Box<dyn FnOnce(&mut Database) + Send>;

/// Async wrapper around [`Database`] using a dedicated OS thread.
///
/// All SQL operations run on a single background thread that owns the
/// `Database` (and its `rusqlite::Connection`). Callers send closures over
/// an `mpsc` channel; results come back via `tokio::sync::oneshot`.
///
/// `Clone` is cheap (clones the inner `Arc` and agent_id string). All clones
/// sharing the same `inner` share the same background thread and connection.
#[derive(Clone)]
pub struct AsyncDatabase {
    inner: Arc<AsyncDatabaseInner>,
    /// The agent context for all operations on this handle (default: "mika").
    pub agent_id: String,
}

struct AsyncDatabaseInner {
    sender: Mutex<Option<tokio::sync::mpsc::Sender<DbClosure>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    /// Per-process task-event broadcast handle (mika#1758). Attached once at
    /// server startup by `run_server` (and `AppState::resolve_agent` for
    /// lazy-resolved agents) via [`AsyncDatabase::set_task_events_channel`].
    /// `OnceLock::get()` returns `None` in tests / CLI / any non-server caller,
    /// making [`AsyncDatabase::emit_task_event`] a silent no-op there. All
    /// clones share the same `Inner`, so a single attach covers every derived
    /// handle (including those returned by [`AsyncDatabase::with_agent`]).
    task_events_channel: OnceLock<Arc<TaskEventsChannel>>,
}

impl AsyncDatabase {
    /// Spawn a dedicated OS thread that owns `db` and processes closures.
    pub fn new(db: Database) -> Self {
        Self::new_with_agent(db, "mika")
    }

    /// Spawn with a specific agent_id.
    pub fn new_with_agent(mut db: Database, agent_id: &str) -> Self {
        // Bounded channel: async backpressure when DB thread falls behind.
        // tokio::sync::mpsc::Sender::send().await yields to the executor
        // instead of blocking the Tokio worker thread (mika#1258).
        let (tx, mut rx) = tokio::sync::mpsc::channel::<DbClosure>(512);
        let handle = std::thread::Builder::new()
            .name("mika-db".to_string())
            .spawn(move || {
                // rusqlite::Connection is !Send, so the worker MUST be a dedicated
                // OS thread — not a tokio task. blocking_recv() is the sync bridge
                // for tokio::sync::mpsc — it blocks the current thread until a
                // message arrives or all senders are dropped.
                while let Some(f) = rx.blocking_recv() {
                    if let Err(_panic) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            f(&mut db);
                        }))
                    {
                        tracing::error!("database closure panicked — thread continues");
                    }
                }
            })
            .expect("failed to spawn database thread");
        Self {
            inner: Arc::new(AsyncDatabaseInner {
                sender: Mutex::new(Some(tx)),
                thread_handle: Mutex::new(Some(handle)),
                task_events_channel: OnceLock::new(),
            }),
            agent_id: agent_id.to_string(),
        }
    }

    /// Attach the per-process task-event broadcast channel (mika#1758). Safe
    /// to call repeatedly — only the first call wins (`OnceLock::set` returns
    /// `Err` on second attach and is silently ignored). Every clone of this
    /// handle shares the same inner and therefore the same attached channel;
    /// a single attach at server startup covers dispatcher/engine/tool
    /// derivations via [`Self::with_agent`] and `.clone()`.
    ///
    /// When unattached (tests / CLI), [`Self::emit_task_event`] is a silent
    /// no-op.
    pub fn set_task_events_channel(&self, channel: Arc<TaskEventsChannel>) {
        let _ = self.inner.task_events_channel.set(channel);
    }

    /// Fire-and-forget broadcast of a [`TaskEventFrame`] (mika#1758).
    ///
    /// **Contract:** callers invoke this AFTER the associated DB write has
    /// committed and returned the "transition happened" signal (`Ok(true)`
    /// for guarded UPDATEs, `Ok(id)` for INSERTs). Absent-channel = silent
    /// no-op. Zero-subscriber broadcast returns `false` — also silent.
    /// This method NEVER errors and NEVER blocks — `broadcast::Sender::send`
    /// is synchronous and lock-free. Do not `.await` — no await point exists.
    fn emit_task_event(&self, frame: TaskEventFrame) {
        if let Some(channel) = self.inner.task_events_channel.get() {
            let _ = channel.broadcast_frame(frame);
        }
    }

    /// Open a database at `path` and wrap it in an async handle (agent_id = "mika").
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::open(path)?;
        Ok(Self::new(db))
    }

    /// Return a clone of this handle scoped to a different agent_id.
    /// The underlying DB thread is shared.
    pub fn with_agent(&self, agent_id: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            agent_id: agent_id.to_string(),
        }
    }

    /// Return the agent_id for this handle.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Gracefully shut down the database thread.
    pub fn shutdown(&self) {
        {
            let mut sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
            *sender_guard = None;
        }
        let mut handle_guard = self
            .inner
            .thread_handle
            .lock()
            .expect("handle lock poisoned");
        if let Some(handle) = handle_guard.take() {
            let _ = handle.join();
        }
    }

    pub async fn with_db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Database) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        // Clone the sender while holding the lock, then release the lock before
        // calling send(). This prevents blocking the tokio worker thread with the
        // mutex held if the channel is full (bounded backpressure).
        let sender = {
            let sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
            sender_guard
                .as_ref()
                .ok_or_else(|| anyhow!("database has been shut down"))?
                .clone()
        };
        sender
            .send(Box::new(move |db| {
                let _ = tx.send(f(db));
            }))
            .await
            .map_err(|_| anyhow!("database thread has stopped"))?;
        rx.await
            .map_err(|_| anyhow!("database thread dropped reply"))?
    }

    // -- Agent CRUD --

    pub async fn register_agent(&self, id: &str, name: &str, home_dir: &str) -> Result<()> {
        let (i, n, h) = (id.to_owned(), name.to_owned(), home_dir.to_owned());
        self.with_db(move |db| db.register_agent(&i, &n, &h)).await
    }

    pub async fn update_agent_last_seen(&self) -> Result<()> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.update_agent_last_seen(&id)).await
    }

    pub async fn list_agents_db(&self) -> Result<Vec<AgentRow>> {
        self.with_db(|db| db.list_agents_db()).await
    }

    // -- Skill Overrides --

    pub async fn get_skill_overrides(&self, agent_id: &str) -> Result<Vec<SkillOverride>> {
        let a = agent_id.to_owned();
        self.with_db(move |db| db.get_skill_overrides(&a)).await
    }

    pub async fn set_skill_override(
        &self,
        agent_id: &str,
        skill_name: &str,
        always_on: bool,
    ) -> Result<()> {
        let (a, s) = (agent_id.to_owned(), skill_name.to_owned());
        self.with_db(move |db| db.set_skill_override(&a, &s, always_on))
            .await
    }

    pub async fn set_skill_enabled(
        &self,
        agent_id: &str,
        skill_name: &str,
        enabled: bool,
    ) -> Result<()> {
        let (a, s) = (agent_id.to_owned(), skill_name.to_owned());
        self.with_db(move |db| db.set_skill_enabled(&a, &s, enabled))
            .await
    }

    pub async fn set_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        let (a, s, st) = (agent_id.to_owned(), skill_name.to_owned(), state.to_owned());
        self.with_db(move |db| db.set_skill_lifecycle_state(&a, &s, &st))
            .await
    }

    pub async fn get_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
    ) -> Result<Option<String>> {
        let (a, s) = (agent_id.to_owned(), skill_name.to_owned());
        self.with_db(move |db| db.get_skill_lifecycle_state(&a, &s))
            .await
    }

    pub async fn delete_skill_override(&self, agent_id: &str, skill_name: &str) -> Result<()> {
        let (a, s) = (agent_id.to_owned(), skill_name.to_owned());
        self.with_db(move |db| db.delete_skill_override(&a, &s))
            .await
    }

    pub async fn increment_skill_usage(
        &self,
        agent_id: &str,
        skill_names: &[String],
    ) -> Result<()> {
        let a = agent_id.to_owned();
        let names = skill_names.to_vec();
        self.with_db(move |db| db.increment_skill_usage(&a, &names))
            .await
    }

    pub async fn get_archival_candidates(
        &self,
        agent_id: &str,
        max_idle_days: u32,
    ) -> Result<Vec<crate::db::SkillOverride>> {
        let a = agent_id.to_owned();
        self.with_db(move |db| db.get_archival_candidates(&a, max_idle_days))
            .await
    }

    pub async fn update_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        let (a, s, st) = (agent_id.to_owned(), skill_name.to_owned(), state.to_owned());
        self.with_db(move |db| db.update_skill_lifecycle_state(&a, &s, &st))
            .await
    }

    // -- Team CRUD --

    pub async fn register_team(&self, id: &str, name: &str, config_path: &str) -> Result<()> {
        let (i, n, c) = (id.to_owned(), name.to_owned(), config_path.to_owned());
        self.with_db(move |db| db.register_team(&i, &n, &c)).await
    }

    pub async fn list_teams_db(&self) -> Result<Vec<TeamRow>> {
        self.with_db(|db| db.list_teams_db()).await
    }

    // -- Task CRUD --

    pub async fn create_task(&self, task: NewTask) -> Result<String> {
        // mika#1758: capture the emission-relevant fields BEFORE moving the
        // NewTask into the DB-thread closure — we cannot read them back after.
        // `created_at` is captured at emit time (off by ≤1ms vs the DB-committed
        // value; the wire is a UI hint, exact-match against DB is not required).
        //
        // agent_id source: `task.agent_id`, NOT `self.agent_id`. The
        // `Database::create_task` INSERT uses `task.agent_id`, and callers on
        // shared handles legitimately pass a different `NewTask.agent_id`
        // (e.g. `teams/engine.rs` writes child tasks with the assigned team
        // member's agent id, not the orchestrator's). The frame's agent_id
        // must match the row on disk so consumers correlating via
        // `GET /api/v1/tasks/{task_id}` see a consistent view.
        let kind = task.trigger_type.clone();
        let action_type = task.action_type.clone();
        let label = if task.label.is_empty() {
            None
        } else {
            Some(task.label.clone())
        };
        let parent_task_id = task.parent_task_id.clone();
        let agent_id = task.agent_id.clone();

        let id = self.with_db(move |db| db.create_task(&task)).await?;

        self.emit_task_event(TaskEventFrame::TaskCreated {
            task_id: id.clone(),
            agent_id,
            kind,
            action_type,
            label,
            parent_task_id,
            created_at: crate::timestamp::now(),
        });

        Ok(id)
    }

    pub async fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        // mika#1758: idempotent create — only emit `TaskCreated` when the
        // underlying DB call returns `Ok(Some(id))` (a new row was inserted).
        // `Ok(None)` means the task already exists, no transition, no frame.
        let kind = task.trigger_type.clone();
        let action_type = task.action_type.clone();
        let label = if task.label.is_empty() {
            None
        } else {
            Some(task.label.clone())
        };
        let parent_task_id = task.parent_task_id.clone();
        let agent_id = task.agent_id.clone();

        let outcome = self
            .with_db(move |db| db.create_recurring_task_if_absent(task))
            .await?;

        if let Some(ref id) = outcome {
            self.emit_task_event(TaskEventFrame::TaskCreated {
                task_id: id.clone(),
                agent_id,
                kind,
                action_type,
                label,
                parent_task_id,
                created_at: crate::timestamp::now(),
            });
        }

        Ok(outcome)
    }

    pub async fn get_recurring_task_cron(&self, label: &str) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        let l = label.to_owned();
        self.with_db(move |db| db.get_recurring_task_cron(&a, &l))
            .await
    }

    pub async fn update_recurring_task_cron(
        &self,
        label: &str,
        new_cron: &str,
        next_fire_at: &str,
    ) -> Result<()> {
        let a = self.agent_id.clone();
        let l = label.to_owned();
        let c = new_cron.to_owned();
        let nf = next_fire_at.to_owned();
        self.with_db(move |db| db.update_recurring_task_cron(&a, &l, &c, &nf))
            .await
    }

    pub async fn cancel_recurring_task_by_label(&self, label: &str) -> Result<()> {
        // mika#1758 note: this method cancels 0..N recurring rows keyed by
        // (agent_id, label) without returning the affected task ids. Emitting
        // per-row `TaskCancelled` frames would require an extra pre-fetch
        // (SELECT id FROM tasks WHERE ...) and doubles the DB round-trip on
        // the cleanup path. Deferred to a follow-up that widens the DB
        // signature to return the affected ids (v2 concern); the wire stays
        // silent on this cleanup transition for v1. The related operator
        // paths (`cancel_task`, `update_manual_task_status`) do emit.
        let a = self.agent_id.clone();
        let l = label.to_owned();
        self.with_db(move |db| db.cancel_recurring_task_by_label(&a, &l))
            .await
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_task(&i, &a)).await
    }

    pub async fn resolve_task_id_by_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        let p = prefix.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.resolve_task_id_by_prefix(&p, &a))
            .await
    }

    pub async fn get_manual_task(&self, id: &str) -> Result<Option<Task>> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_manual_task(&i, &a)).await
    }

    /// Walk the parent_task_id chain to the nearest scope root (mika#974).
    pub async fn resolve_scope_root_task_id(&self, task_id: &str) -> Result<Option<String>> {
        let tid = task_id.to_owned();
        self.with_db(move |db| db.resolve_scope_root_task_id(&tid))
            .await
    }

    pub async fn get_pending_callbacks_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        let s = session_id.to_owned();
        self.with_db(move |db| db.get_pending_callbacks_for_session(&s))
            .await
    }

    pub async fn count_child_tasks(&self, parent_task_id: &str) -> Result<Vec<(String, i64)>> {
        let p = parent_task_id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_child_tasks(&p, &a)).await
    }

    pub async fn get_schedulable_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_schedulable_tasks(&id)).await
    }

    pub async fn claim_and_fire_task(&self, id: &str) -> Result<bool> {
        let id_owned = id.to_string();
        let a = self.agent_id.clone();
        let claimed = self
            .with_db({
                let id_c = id_owned.clone();
                let a_c = a.clone();
                move |db| db.claim_and_fire_task(&id_c, &a_c)
            })
            .await?;

        // mika#1758: emit TaskClaimed only when the atomic claim actually
        // transitioned the row. `Ok(false)` = task no longer claimable
        // (cancelled/completed/expired between DB scan and this call) — no
        // transition, no frame.
        if claimed {
            self.emit_task_event(TaskEventFrame::TaskClaimed {
                task_id: id_owned,
                agent_id: a,
                claimed_at: crate::timestamp::now(),
            });
        }

        Ok(claimed)
    }

    pub async fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        let (i, s) = (id.to_owned(), status.to_owned());
        let is_cancel = s == "cancelled";

        // mika#1758: `Database::update_task_status` is an *unguarded* UPDATE
        // returning `Ok(())` regardless of whether the row existed or the
        // status actually changed. To honour the "emit only on real
        // transitions" contract shared with the guarded wrappers
        // (`cancel_task`, `update_manual_task_status`), pre-fetch the task
        // when a cancel emit would otherwise fire and only emit when the
        // row exists AND was not already `cancelled`. Fail-closed on the
        // pre-fetch error path — treat unknown as already-cancelled so a
        // read failure does not fire a spurious frame.
        let prior_status = if is_cancel {
            match self.get_task_unscoped(&i).await {
                Ok(Some(t)) => Some(t.status),
                Ok(None) => None, // row does not exist → skip emit
                Err(_) => Some("cancelled".to_string()), // read failed → skip emit
            }
        } else {
            None
        };

        self.with_db(move |db| db.update_task_status(&i, &s))
            .await?;

        // Emit TaskCancelled only when a real row transitioned from a
        // non-cancelled state to cancelled. Real caller today:
        // `teams/engine.rs` cancelling a team-parent task. Dedicated cancel
        // wrappers (`cancel_task`, `update_manual_task_status`) own their
        // own emissions and never route through this path for the same
        // call site.
        if is_cancel && prior_status.as_deref().is_some_and(|s| s != "cancelled") {
            self.emit_task_event(TaskEventFrame::TaskCancelled {
                task_id: id.to_owned(),
                agent_id: self.agent_id.clone(),
                cancelled_at: crate::timestamp::now(),
                reason: Some("cancelled_via_update_task_status".to_string()),
            });
        }

        Ok(())
    }

    /// Record the trace_id of the execution that ran this task.
    /// Does NOT scope by agent_id — safe for cross-agent team tasks.
    pub async fn update_task_execution_trace_id(&self, id: &str, trace_id: &str) -> Result<()> {
        let (i, t) = (id.to_owned(), trace_id.to_owned());
        self.with_db(move |db| db.update_task_execution_trace_id(&i, &t))
            .await
    }

    pub async fn update_task_completed(&self, id: &str, result: Option<&str>) -> Result<bool> {
        let id_owned = id.to_owned();
        let result_owned = result.map(|s| s.to_owned());
        let a = self.agent_id.clone();
        let transitioned = self
            .with_db({
                let i = id_owned.clone();
                let a_c = a.clone();
                let r = result_owned.clone();
                move |db| db.update_task_completed(&i, &a_c, r.as_deref())
            })
            .await?;

        // mika#1758: emit TaskCompleted only when the guarded UPDATE actually
        // transitioned the row. `Ok(false)` = task already in terminal state
        // (race with reaper/promoter/operator) — no transition, no frame.
        if transitioned {
            self.emit_task_event(TaskEventFrame::completed(
                id_owned,
                a,
                crate::timestamp::now(),
                result_owned.as_deref(),
            ));
        }

        Ok(transitioned)
    }

    pub async fn update_task_failed(&self, id: &str, error: &str) -> Result<bool> {
        let id_owned = id.to_string();
        let error_owned = error.to_string();
        let a = self.agent_id.clone();
        let transitioned = self
            .with_db({
                let i = id_owned.clone();
                let a_c = a.clone();
                let e = error_owned.clone();
                move |db| db.update_task_failed(&i, &a_c, &e)
            })
            .await?;

        // mika#1758: emit TaskFailed only when the guarded UPDATE actually
        // transitioned the row. `Ok(false)` = task already terminal — no frame.
        if transitioned {
            self.emit_task_event(TaskEventFrame::failed(
                id_owned,
                a,
                crate::timestamp::now(),
                Some(error_owned.as_str()),
            ));
        }

        Ok(transitioned)
    }

    /// Promote a task from `failed` → `completed` (#958).
    pub async fn promote_task_completed(&self, id: &str, reason: &str) -> Result<bool> {
        let id_owned = id.to_string();
        let reason_owned = reason.to_string();
        let a = self.agent_id.clone();
        let transitioned = self
            .with_db({
                let i = id_owned.clone();
                let a_c = a.clone();
                let r = reason_owned.clone();
                move |db| db.promote_task_completed(&i, &a_c, &r)
            })
            .await?;

        // mika#1758: emit TaskCompleted on the failed→completed promotion.
        // From the observer's standpoint, "task reached completed" is the
        // semantic invariant — the intermediate failed → completed promotion
        // is a legitimate second `TaskCompleted` for the same task_id. The
        // wire faithfully records the sequence; consumers who only care about
        // final state can ignore the earlier `TaskFailed`.
        if transitioned {
            self.emit_task_event(TaskEventFrame::completed(
                id_owned,
                a,
                crate::timestamp::now(),
                Some(reason_owned.as_str()),
            ));
        }

        Ok(transitioned)
    }

    pub async fn update_task_next_fire_at(&self, id: &str, next_fire_at: &str) -> Result<()> {
        let (i, nf) = (id.to_owned(), next_fire_at.to_owned());
        self.with_db(move |db| db.update_task_next_fire_at(&i, &nf))
            .await
    }

    pub async fn update_task_rescheduled(&self, id: &str, next_fire_at: &str) -> Result<()> {
        let i = id.to_owned();
        let nf = next_fire_at.to_owned();
        self.with_db(move |db| db.update_task_rescheduled(&i, &nf))
            .await
    }

    pub async fn cancel_task(&self, id: &str) -> Result<bool> {
        let id_owned = id.to_owned();
        let a = self.agent_id.clone();
        let cancelled = self
            .with_db({
                let i = id_owned.clone();
                let a_c = a.clone();
                move |db| db.cancel_task(&i, &a_c)
            })
            .await?;

        // mika#1758: emit TaskCancelled only when the guarded UPDATE actually
        // transitioned the row. `Ok(false)` = task not in a cancellable state.
        if cancelled {
            self.emit_task_event(TaskEventFrame::TaskCancelled {
                task_id: id_owned,
                agent_id: a,
                cancelled_at: crate::timestamp::now(),
                reason: Some("cancelled_via_cancel_task".to_string()),
            });
        }

        Ok(cancelled)
    }

    pub async fn update_manual_task_status(
        &self,
        task_id: &str,
        new_status: &str,
    ) -> Result<Option<String>> {
        let id_owned = task_id.to_owned();
        let a = self.agent_id.clone();
        let status_owned = new_status.to_owned();
        let is_cancel = status_owned == "cancelled";
        let outcome = self
            .with_db({
                let i = id_owned.clone();
                let a_c = a.clone();
                let s = status_owned.clone();
                move |db| db.update_manual_task_status(&i, &a_c, &s)
            })
            .await?;

        // mika#1758: emit TaskCancelled when the manual-task-status wrapper
        // transitions to "cancelled". `Ok(Some(prior_status))` means the
        // guarded UPDATE actually fired; `Ok(None)` = no-op.
        if is_cancel && outcome.is_some() {
            self.emit_task_event(TaskEventFrame::TaskCancelled {
                task_id: id_owned,
                agent_id: a,
                cancelled_at: crate::timestamp::now(),
                reason: Some("cancelled_via_update_manual_task_status".to_string()),
            });
        }

        Ok(outcome)
    }

    pub async fn list_manual_tasks(
        &self,
        status_filter: Option<&str>,
        source_filter: Option<&str>,
        include_children: bool,
    ) -> Result<Vec<(Task, Option<i64>)>> {
        let a = self.agent_id.clone();
        let sf = status_filter.map(|s| s.to_owned());
        let src = source_filter.map(|s| s.to_owned());
        self.with_db(move |db| {
            db.list_manual_tasks(&a, sf.as_deref(), src.as_deref(), include_children)
        })
        .await
    }

    pub async fn count_session_tasks(&self, session_id: &str) -> Result<i64> {
        let a = self.agent_id.clone();
        let s = session_id.to_owned();
        self.with_db(move |db| db.count_session_tasks(&a, &s)).await
    }

    /// Count audit_events for (tool_name, target_key) with created_at > since (ISO 8601 UTC).
    /// Used by the PR-keyed circuit breaker in `verdict_handler` (mika#1563).
    pub async fn count_recent_audit_events_for_target(
        &self,
        tool_name: &str,
        target_key: &str,
        since: &str,
    ) -> Result<i64> {
        let a = self.agent_id.clone();
        let tn = tool_name.to_owned();
        let tk = target_key.to_owned();
        let sn = since.to_owned();
        self.with_db(move |db| db.count_recent_audit_events_for_target(&a, &tn, &tk, &sn))
            .await
    }

    pub async fn find_active_task_by_ref_url(&self, reference_url: &str) -> Result<Option<Task>> {
        let a = self.agent_id.clone();
        let url = reference_url.to_owned();
        self.with_db(move |db| db.find_active_task_by_ref_url(&a, &url))
            .await
    }

    pub async fn find_active_task_by_pr_url(&self, pr_url: &str) -> Result<Option<Task>> {
        let a = self.agent_id.clone();
        let url = pr_url.to_owned();
        self.with_db(move |db| db.find_active_task_by_pr_url(&a, &url))
            .await
    }

    pub async fn find_active_task_by_branch(&self, branch: &str) -> Result<Option<Task>> {
        let a = self.agent_id.clone();
        let b = branch.to_owned();
        self.with_db(move |db| db.find_active_task_by_branch(&a, &b))
            .await
    }

    pub async fn find_active_task_by_label(&self, label: &str) -> Result<Option<Task>> {
        let a = self.agent_id.clone();
        let l = label.to_owned();
        self.with_db(move |db| db.find_active_task_by_label(&a, &l))
            .await
    }

    pub async fn get_task_depth(&self, task_id: &str) -> Result<Option<i64>> {
        let a = self.agent_id.clone();
        let i = task_id.to_owned();
        self.with_db(move |db| db.get_task_depth(&i, &a)).await
    }

    pub async fn list_active_tasks(&self) -> Result<Vec<Task>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.list_active_tasks(&a)).await
    }

    pub async fn get_task_health_summary(&self) -> Result<TaskHealthSummary> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_task_health_summary(&a)).await
    }

    pub async fn mark_tasks_expired(&self, now: &str) -> Result<usize> {
        let id = self.agent_id.clone();
        let n = now.to_owned();
        self.with_db(move |db| db.mark_tasks_expired(&n, &id)).await
    }

    pub async fn get_expired_child_task_ids(&self) -> Result<Vec<String>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_expired_child_task_ids(&id))
            .await
    }

    pub async fn count_pending_tasks(&self) -> Result<i64> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.count_pending_tasks(&id)).await
    }

    /// Count active self_dev tasks (mika#1363 F2).
    pub async fn count_active_self_dev_tasks(&self) -> Result<i64> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.count_active_self_dev_tasks(&id))
            .await
    }

    /// True if an active self_dev task references this issue (mika#1824 D6).
    pub async fn has_active_self_dev_task_for_issue(&self, issue_url: &str) -> Result<bool> {
        let id = self.agent_id.clone();
        let url = issue_url.to_owned();
        self.with_db(move |db| db.has_active_self_dev_task_for_issue(&id, &url))
            .await
    }

    /// Get auto-pull failure count for circuit-breaker (mika#1363).
    pub async fn get_auto_pull_failure_count(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<i64> {
        let repo = repo_full_name.to_owned();
        self.with_db(move |db| db.get_auto_pull_failure_count(&repo, issue_number))
            .await
    }

    /// Record an auto-pull event (mika#1363).
    pub async fn record_auto_pull(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        let repo = repo_full_name.to_owned();
        self.with_db(move |db| db.record_auto_pull(&repo, issue_number))
            .await
    }

    /// Increment auto-pull failure counter (mika#1363).
    pub async fn increment_auto_pull_failure(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let repo = repo_full_name.to_owned();
        self.with_db(move |db| db.increment_auto_pull_failure(&repo, issue_number))
            .await
    }

    /// Reset auto-pull failure counter (mika#1363).
    pub async fn reset_auto_pull_failure(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let repo = repo_full_name.to_owned();
        self.with_db(move |db| db.reset_auto_pull_failure(&repo, issue_number))
            .await
    }

    pub async fn get_user_visible_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_user_visible_tasks(&id)).await
    }

    pub async fn get_background_task_counts(&self) -> Result<BackgroundTaskCounts> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_background_task_counts(&id))
            .await
    }

    pub async fn get_active_background_task_count(&self) -> Result<usize> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_active_background_task_count(&id))
            .await
    }

    pub async fn get_inject_context_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_inject_context_tasks(&id))
            .await
    }

    pub async fn get_undelivered_callback_tasks(&self, since: &str) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        let s = since.to_owned();
        self.with_db(move |db| db.get_undelivered_callback_tasks(&id, &s))
            .await
    }

    pub async fn get_undelivered_callback_tasks_for_session(
        &self,
        since: &str,
        session_id: &str,
    ) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        let s = since.to_owned();
        let sid = session_id.to_owned();
        self.with_db(move |db| db.get_undelivered_callback_tasks_for_session(&id, &s, &sid))
            .await
    }

    pub async fn mark_task_delivered(&self, task_id: &str) -> Result<bool> {
        let id_owned = task_id.to_owned();
        let a = self.agent_id.clone();
        let delivered = self
            .with_db({
                let i = id_owned.clone();
                move |db| db.mark_task_delivered(&i)
            })
            .await?;

        // mika#1758: emit TaskDelivered only when the guarded UPDATE actually
        // transitioned the row (status='completed' AND delivered_at IS NULL).
        // `Ok(false)` = already delivered or not yet completed — no frame.
        if delivered {
            self.emit_task_event(TaskEventFrame::TaskDelivered {
                task_id: id_owned,
                agent_id: a,
                delivered_at: crate::timestamp::now(),
            });
        }

        Ok(delivered)
    }

    /// Find orphaned parent self_dev tasks whose callback subtask delivered
    /// without producing a PR. See [`Database::find_orphaned_parent_tasks`].
    pub async fn find_orphaned_parent_tasks(
        &self,
        grace_seconds: i64,
    ) -> Result<Vec<crate::db::OrphanedParentTask>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.find_orphaned_parent_tasks(&a, grace_seconds))
            .await
    }

    /// Find phantom tracking rows (`action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`, aged past `age_seconds`) for the
    /// scoped agent. See [`Database::find_phantom_tracking_tasks`]. mika#1712.
    pub async fn find_phantom_tracking_tasks(
        &self,
        age_seconds: i64,
    ) -> Result<Vec<crate::db::PhantomTrackingTask>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.find_phantom_tracking_tasks(&a, age_seconds))
            .await
    }

    /// Count `audit_events` rows for the scoped agent + `tool_name`. Wraps
    /// [`Database::count_audit_events_by_tool_name`]. Used by mika#1712
    /// integration tests to assert the load-bearing audit-write delta on the
    /// phantom sweep path.
    #[doc(hidden)]
    pub async fn count_audit_events_by_tool_name(&self, tool_name: &str) -> Result<i64> {
        let a = self.agent_id.clone();
        let tn = tool_name.to_owned();
        self.with_db(move |db| db.count_audit_events_by_tool_name(&a, &tn))
            .await
    }

    /// Fetch the `target_key` values of `audit_events` rows for the scoped
    /// agent + `tool_name`. Wraps
    /// [`Database::get_audit_event_target_keys_by_tool_name`]. Used by mika#1712
    /// integration tests for row-shape assertions.
    #[doc(hidden)]
    pub async fn get_audit_event_target_keys_by_tool_name(
        &self,
        tool_name: &str,
    ) -> Result<Vec<String>> {
        let a = self.agent_id.clone();
        let tn = tool_name.to_owned();
        self.with_db(move |db| db.get_audit_event_target_keys_by_tool_name(&a, &tn))
            .await
    }

    /// Fetch full `audit_events` rows (target_key, before, after, reasoning)
    /// for the scoped agent + `tool_name`. Wraps
    /// [`Database::get_audit_event_rows_by_tool_name`]. Used by mika#1712
    /// integration tests for the full-row-shape assertion (T-1/T-2).
    #[doc(hidden)]
    pub async fn get_audit_event_rows_by_tool_name(
        &self,
        tool_name: &str,
    ) -> Result<Vec<crate::db::AuditEventRowTuple>> {
        let a = self.agent_id.clone();
        let tn = tool_name.to_owned();
        self.with_db(move |db| db.get_audit_event_rows_by_tool_name(&a, &tn))
            .await
    }

    /// Test-only wrapper for [`Database::backdate_task_updated_at`] (mika#1712).
    /// Not for production use.
    #[doc(hidden)]
    pub async fn backdate_task_updated_at(&self, task_id: &str, seconds_ago: i64) -> Result<()> {
        let t = task_id.to_owned();
        self.with_db(move |db| db.backdate_task_updated_at(&t, seconds_ago))
            .await
    }

    /// Find completable parent self_dev tasks whose callback subtask delivered
    /// WITH a `pr_url` (success indicator) but were never transitioned by the
    /// silent agent turn (mika#1162). See
    /// [`Database::find_completable_parent_tasks_on_pr_url`].
    pub async fn find_completable_parent_tasks_on_pr_url(
        &self,
        grace_seconds: i64,
    ) -> Result<Vec<crate::db::CompletableParentTask>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.find_completable_parent_tasks_on_pr_url(&a, grace_seconds))
            .await
    }

    /// Find parent self_dev issue tasks left `in_progress` with zero callback
    /// children, aged past the grace window (mika#1687). See
    /// [`Database::find_childless_stuck_parent_tasks`].
    pub async fn find_childless_stuck_parent_tasks(
        &self,
        grace_seconds: i64,
    ) -> Result<Vec<crate::db::ChildlessStuckParent>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.find_childless_stuck_parent_tasks(&a, grace_seconds))
            .await
    }

    /// True when the parent still has a `pending` deferred wrapper representing
    /// it (mika#2045). See [`Database::has_pending_deferred_wrapper_child`].
    pub async fn has_pending_deferred_wrapper_child(&self, parent_task_id: &str) -> Result<bool> {
        let a = self.agent_id.clone();
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.has_pending_deferred_wrapper_child(&a, &p))
            .await
    }

    /// Find `pending` self_dev issue parents that no callback child represents
    /// any more (mika#2045). See [`Database::find_orphaned_pending_issue_tasks`].
    pub async fn find_orphaned_pending_issue_tasks(
        &self,
        grace_seconds: i64,
    ) -> Result<Vec<crate::db::OrphanedPendingTask>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.find_orphaned_pending_issue_tasks(&a, grace_seconds))
            .await
    }

    /// Read `metadata.stuck_rearm_count` (mika#2045).
    /// See [`Database::get_stuck_rearm_count`].
    pub async fn get_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        let t = task_id.to_owned();
        self.with_db(move |db| db.get_stuck_rearm_count(&t)).await
    }

    /// Increment `metadata.stuck_rearm_count` (mika#2045).
    /// See [`Database::increment_stuck_rearm_count`].
    pub async fn increment_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        let t = task_id.to_owned();
        self.with_db(move |db| db.increment_stuck_rearm_count(&t))
            .await
    }

    /// Cancel a parent's surviving deferred wrappers before expiry (mika#2045).
    /// See [`Database::cancel_deferred_wrappers_of_parent`].
    pub async fn cancel_deferred_wrappers_of_parent(&self, parent_task_id: &str) -> Result<usize> {
        let a = self.agent_id.clone();
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.cancel_deferred_wrappers_of_parent(&a, &p))
            .await
    }

    /// The `action_config` of a parent's most recent deferred wrapper
    /// (mika#2045). See [`Database::latest_deferred_wrapper_action_config`].
    pub async fn latest_deferred_wrapper_action_config(
        &self,
        parent_task_id: &str,
    ) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.latest_deferred_wrapper_action_config(&a, &p))
            .await
    }

    /// Return ALL children of a parent task for the reaper's structured log
    /// event. See [`Database::get_reaper_child_snapshot`].
    pub async fn get_reaper_child_snapshot(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<crate::db::ReaperChildSnapshot>> {
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.get_reaper_child_snapshot(&p))
            .await
    }

    pub async fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        let i = id.to_owned();
        self.with_db(move |db| db.set_task_process_id(&i, process_id))
            .await
    }

    pub async fn get_expired_tasks_with_process_id(&self) -> Result<Vec<(String, i64)>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_expired_tasks_with_process_id(&id))
            .await
    }

    pub async fn clear_task_process_id(&self, id: &str) -> Result<()> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.clear_task_process_id(&i, &a))
            .await
    }

    /// Get active callback tasks with a process_id set (#959).
    pub async fn get_active_callback_tasks_with_pid(&self) -> Result<Vec<crate::db::Task>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_active_callback_tasks_with_pid(&a))
            .await
    }

    /// Set a single field in the task's metadata JSON (#959).
    pub async fn set_task_metadata_field(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let (i, k, v) = (task_id.to_owned(), key.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_task_metadata_field(&i, &k, &v))
            .await
    }

    /// Get a single metadata field from a task's metadata JSON.
    pub async fn get_task_metadata_field(
        &self,
        task_id: &str,
        key: &str,
    ) -> Result<Option<String>> {
        let (i, k) = (task_id.to_owned(), key.to_owned());
        self.with_db(move |db| db.get_task_metadata_field(&i, &k))
            .await
    }

    /// Remove a single field from the task's metadata JSON (#959).
    pub async fn remove_task_metadata_field(&self, task_id: &str, key: &str) -> Result<()> {
        let (i, k) = (task_id.to_owned(), key.to_owned());
        self.with_db(move |db| db.remove_task_metadata_field(&i, &k))
            .await
    }

    pub async fn try_complete_parent_on_sibling_done(
        &self,
        task_id: &str,
    ) -> Result<Option<String>> {
        let i = task_id.to_owned();
        self.with_db(move |db| db.try_complete_parent_on_sibling_done(&i))
            .await
    }

    pub async fn get_child_tasks(&self, parent_task_id: &str) -> Result<Vec<Task>> {
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.get_child_tasks(&p)).await
    }

    pub async fn get_task_descendants(&self, root_task_id: &str) -> Result<Vec<Task>> {
        let r = root_task_id.to_owned();
        self.with_db(move |db| db.get_task_descendants(&r)).await
    }

    /// Returns `(parent_task_id, callback_id, callback_label)` of the blocking
    /// callback, or `None` if no conflicting dispatch exists (#1172 W3).
    pub async fn has_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<(String, String, String)>> {
        let p = excluded_parent_id.to_owned();
        let a = self.agent_id.clone();
        let c = dispatch_class.to_owned();
        self.with_db(move |db| db.has_active_callback_tasks_excluding(&p, &a, &c))
            .await
    }

    pub async fn update_task_dispatch_class(&self, id: &str, dispatch_class: &str) -> Result<bool> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        let c = dispatch_class.to_owned();
        self.with_db(move |db| db.update_task_dispatch_class(&i, &a, &c))
            .await
    }

    /// Write a dispatch-rejection reason to `tasks.result` without changing status (#1108).
    pub async fn write_task_dispatch_rejection(
        &self,
        task_id: &str,
        reason_json: &str,
    ) -> Result<bool> {
        let i = task_id.to_owned();
        let r = reason_json.to_owned();
        self.with_db(move |db| db.write_task_dispatch_rejection(&i, &r))
            .await
    }

    pub async fn count_pending_callback_tasks_by_team_run(&self, team_run_id: &str) -> Result<i64> {
        let r = team_run_id.to_owned();
        self.with_db(move |db| db.count_pending_callback_tasks_by_team_run(&r))
            .await
    }

    /// Check if a parent task has any active non-deferred callback child (#1172 R9).
    pub async fn has_non_deferred_active_callback_child(
        &self,
        parent_task_id: &str,
    ) -> Result<bool> {
        let p = parent_task_id.to_owned();
        self.with_db(move |db| db.has_non_deferred_active_callback_child(&p))
            .await
    }

    /// Check whether a completed groom-class task exists for a given GitHub
    /// issue (#1620). Used by the dispatch-classification gate to verify
    /// grooming markers were written by the autonomous loop.
    pub async fn has_completed_groom_for_issue(&self, issue_url: &str) -> Result<bool> {
        let a = self.agent_id.clone();
        let u = issue_url.to_owned();
        self.with_db(move |db| db.has_completed_groom_for_issue(&a, &u))
            .await
    }

    /// Count pending deferred-dispatch callbacks for this agent (mika#1011).
    pub async fn count_pending_deferred_callbacks(&self) -> Result<i64> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_pending_deferred_callbacks(&a))
            .await
    }

    /// Promote the next pending deferred-dispatch callback for dispatch (mika#1011).
    /// Returns `Some(task_id)` of the promoted task, or `None` if no wrapper pending.
    pub async fn promote_next_deferred_callback(&self) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.promote_next_deferred_callback(&a))
            .await
    }

    /// Class-scoped sibling of `promote_next_deferred_callback` (mika#1175).
    /// Returns `Some(task_id)` of the promoted task, or `None` if no wrapper pending.
    pub async fn promote_next_deferred_callback_for_class(
        &self,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        let c = dispatch_class.to_string();
        self.with_db(move |db| db.promote_next_deferred_callback_for_class(&a, &c))
            .await
    }

    /// Check if any non-deferred callback task is active (mika#1070).
    pub async fn has_any_active_callback(&self) -> Result<bool> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.has_any_active_callback(&a)).await
    }

    /// Class-scoped sibling of `has_any_active_callback` (mika#1175).
    pub async fn has_any_active_callback_for_class(&self, dispatch_class: &str) -> Result<bool> {
        let a = self.agent_id.clone();
        let c = dispatch_class.to_string();
        self.with_db(move |db| db.has_any_active_callback_for_class(&a, &c))
            .await
    }

    /// Force-promote the next pending deferred wrapper for a dispatch class
    /// with fail-closed slot-availability semantics (mika#1453).
    pub async fn force_promote_deferred_for_class(
        &self,
        dispatch_class: &str,
    ) -> Result<crate::db::ForcePromoteResult> {
        let a = self.agent_id.clone();
        let c = dispatch_class.to_string();
        self.with_db(move |db| db.force_promote_deferred_for_class(&a, &c))
            .await
    }

    /// Returns the task ID of the active non-deferred callback occupying the
    /// per-class dispatch slot. Used by the CLI override path (mika#1453).
    pub async fn find_active_callback_for_class(
        &self,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        let c = dispatch_class.to_string();
        self.with_db(move |db| db.find_active_callback_for_class(&a, &c))
            .await
    }

    pub async fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        self.with_db(move |db| db.prune_completed_tasks(older_than_secs))
            .await
    }

    pub async fn get_tasks_by_status(&self, statuses: Vec<String>) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| {
            let refs: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
            db.get_tasks_by_status(&id, &refs)
        })
        .await
    }

    /// Like `get_tasks_by_status`, but with an optional label substring filter (#1172 W1).
    pub async fn get_tasks_by_status_and_label(
        &self,
        statuses: Vec<String>,
        label_contains: Option<String>,
    ) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| {
            let refs: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
            db.get_tasks_by_status_and_label(&id, &refs, label_contains.as_deref())
        })
        .await
    }

    // -- Sessions --

    pub async fn create_session(&self, id: &str, agent_id: &str, channel_type: &str) -> Result<()> {
        let (i, a, ct) = (id.to_owned(), agent_id.to_owned(), channel_type.to_owned());
        self.with_db(move |db| db.create_session(&i, &a, &ct)).await
    }

    pub async fn create_session_with_metadata(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        let (i, a, ct, m, t) = (
            id.to_owned(),
            agent_id.to_owned(),
            channel_type.to_owned(),
            metadata.map(|s| s.to_owned()),
            task_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.create_session_with_metadata(&i, &a, &ct, m.as_deref(), t.as_deref())
        })
        .await
    }

    /// Create a session with metadata, parent session reference, and optional task linkage.
    pub async fn create_session_with_parent(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        parent_session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        let (i, a, ct, m, p, t) = (
            id.to_owned(),
            agent_id.to_owned(),
            channel_type.to_owned(),
            metadata.map(|s| s.to_owned()),
            parent_session_id.map(|s| s.to_owned()),
            task_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.create_session_with_parent(&i, &a, &ct, m.as_deref(), p.as_deref(), t.as_deref())
        })
        .await
    }

    /// Create a session with metadata if it doesn't already exist (INSERT OR IGNORE).
    pub async fn create_session_if_not_exists(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        let (i, a, ct, m) = (
            id.to_owned(),
            agent_id.to_owned(),
            channel_type.to_owned(),
            metadata.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.create_session_if_not_exists(&i, &a, &ct, m.as_deref()))
            .await
    }

    pub async fn end_session(&self, id: &str) -> Result<()> {
        let i = id.to_owned();
        self.with_db(move |db| db.end_session(&i)).await
    }

    /// End a session unless it is the agent's canonical singleton session (mika#1401).
    /// No-ops when `id == canonical_id`; otherwise behaves like [`end_session`].
    pub async fn end_session_unless_canonical(
        &self,
        id: &str,
        canonical_id: Option<&str>,
    ) -> Result<()> {
        let (i, c) = (id.to_owned(), canonical_id.map(|s| s.to_owned()));
        self.with_db(move |db| db.end_session_unless_canonical(&i, c.as_deref()))
            .await
    }

    pub async fn get_or_create_system_session(&self) -> Result<String> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_or_create_system_session(&a))
            .await
    }

    /// Idempotently create the canonical singleton session (mika#1401).
    /// `INSERT OR IGNORE` — safe to call on every invocation.
    pub async fn get_or_create_canonical_session(
        &self,
        session_id: &str,
        channel_type: &str,
    ) -> Result<String> {
        let (s, a, ct) = (
            session_id.to_owned(),
            self.agent_id.clone(),
            channel_type.to_owned(),
        );
        self.with_db(move |db| db.get_or_create_canonical_session(&s, &a, &ct))
            .await
    }

    /// Prune ended system/silent sessions older than `retention_secs`.
    pub async fn prune_old_sessions(&self, retention_secs: i64) -> Result<usize> {
        self.with_db(move |db| db.prune_old_sessions(retention_secs))
            .await
    }

    // -- Messages --

    pub async fn save_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        let (a, sid, r, c, t) = (
            self.agent_id.clone(),
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.save_message(&a, &sid, &r, &c, t.as_deref()))
            .await
    }

    pub async fn save_message_with_metadata(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
    ) -> Result<i64> {
        let (a, sid, r, c, m, t) = (
            self.agent_id.clone(),
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
            metadata.map(|s| s.to_owned()),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.save_message_with_metadata(&a, &sid, &r, &c, m.as_deref(), t.as_deref(), internal)
        })
        .await
    }

    /// Double-write: insert into `messages` AND `task_messages` in a single transaction.
    /// When `task_id` is `None`, behaves identically to `save_message_with_metadata`.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_message_with_task_context(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
        task_id: Option<&str>,
    ) -> Result<i64> {
        let (a, sid, r, c, m, t, tid) = (
            self.agent_id.clone(),
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
            metadata.map(|s| s.to_owned()),
            trace_id.map(|s| s.to_owned()),
            task_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.save_message_with_task_context(
                &a,
                &sid,
                &r,
                &c,
                m.as_deref(),
                t.as_deref(),
                internal,
                tid.as_deref(),
            )
        })
        .await
    }

    /// Insert a single row into `task_messages` without a transaction (mika#965).
    /// Used by the dispatcher for engine-internal task narrative that should NOT
    /// appear in `messages`.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_task_message(
        &self,
        task_id: &str,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        let (tid, aid, sid, r, c, m, t) = (
            task_id.to_owned(),
            agent_id.to_owned(),
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
            metadata.map(|s| s.to_owned()),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.insert_task_message(&tid, &aid, &sid, &r, &c, m.as_deref(), t.as_deref())
        })
        .await
    }

    /// Load all task messages for a given task, ordered by creation time.
    pub async fn load_task_messages(&self, task_id: &str) -> Result<Vec<TaskMessage>> {
        let tid = task_id.to_owned();
        self.with_db(move |db| db.load_task_messages(&tid)).await
    }

    /// Save a message with an explicit `agent_id` (instead of using `self.agent_id`).
    ///
    /// Used by the team engine where the calling DB handle belongs to the orchestrator
    /// but the message was produced by a different agent.
    pub async fn save_message_as(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        let (a, sid, r, c, m, t) = (
            agent_id.to_owned(),
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
            metadata.map(|s| s.to_owned()),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.save_message_with_metadata(&a, &sid, &r, &c, m.as_deref(), t.as_deref(), false)
        })
        .await
    }

    pub async fn load_recent_messages(&self, limit: usize) -> Result<Vec<SessionMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.load_recent_messages(&a, limit))
            .await
    }

    /// Rebuild conversation context with optional task-mode hybrid merge (mika#974).
    pub async fn rebuild_context(
        &self,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let a = self.agent_id.clone();
        let tid = task_id.map(|s| s.to_owned());
        self.with_db(move |db| db.rebuild_context(&a, tid.as_deref(), limit))
            .await
    }

    /// Load recent messages with optional internal-message filtering.
    ///
    /// Returns `(visible_messages, hidden_internal_count)`. See
    /// `Database::load_recent_messages_filtered` for semantics.
    pub async fn load_recent_messages_filtered(
        &self,
        limit: usize,
        exclude_internal: bool,
    ) -> Result<(Vec<SessionMessage>, usize)> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.load_recent_messages_filtered(&a, limit, exclude_internal))
            .await
    }

    pub async fn load_conversation_summary(&self) -> Result<Option<SessionMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.load_conversation_summary(&a))
            .await
    }

    pub async fn count_messages(&self) -> Result<usize> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_messages(&a)).await
    }

    pub async fn load_messages_before_window(
        &self,
        window_size: usize,
    ) -> Result<Vec<SessionMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.load_messages_before_window(&a, window_size))
            .await
    }

    pub async fn replace_with_summary(
        &self,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let (a, s) = (self.agent_id.clone(), summary.to_owned());
        self.with_db(move |db| db.replace_with_summary(&a, &s, compacted_through_id))
            .await
    }

    // -- Core Memory --

    pub async fn get_core_memory(&self, key: &str) -> Result<Option<CoreMemoryEntry>> {
        let (a, k) = (self.agent_id.clone(), key.to_owned());
        self.with_db(move |db| db.get_core_memory(&a, &k)).await
    }

    pub async fn get_all_core_memory(&self) -> Result<Vec<CoreMemoryEntry>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_all_core_memory(&a)).await
    }

    pub async fn set_core_memory(&self, key: &str, value: &str) -> Result<i32> {
        let (a, k, v) = (self.agent_id.clone(), key.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_core_memory(&a, &k, &v)).await
    }

    pub async fn get_agent_display_name(&self) -> String {
        let a = self.agent_id.clone();
        self.with_db(move |db| Ok(db.get_agent_display_name(&a)))
            .await
            .unwrap_or_else(|_| self.agent_id.clone())
    }

    pub async fn seed_core_memory(&self, user_md_content: Option<String>) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.seed_core_memory(&a, user_md_content.as_deref()))
            .await
    }

    pub async fn migrate_persona_to_self_model(&self) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.migrate_persona_to_self_model(&a))
            .await
    }

    pub async fn total_core_memory_tokens(&self) -> Result<i32> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.total_core_memory_tokens(&a))
            .await
    }

    // -- People --

    pub async fn upsert_person(
        &self,
        name: &str,
        relationship: Option<&str>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let (a, n, r, no) = (
            self.agent_id.clone(),
            name.to_owned(),
            relationship.map(|s| s.to_owned()),
            notes.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.upsert_person(&a, &n, r.as_deref(), no.as_deref()))
            .await
    }

    pub async fn get_person(&self, name: &str) -> Result<Option<Person>> {
        let (a, n) = (self.agent_id.clone(), name.to_owned());
        self.with_db(move |db| db.get_person(&a, &n)).await
    }

    pub async fn list_people(&self) -> Result<Vec<Person>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.list_people(&a)).await
    }

    pub async fn search_people(&self, query: &str) -> Result<Vec<Person>> {
        let (a, q) = (self.agent_id.clone(), query.to_owned());
        self.with_db(move |db| db.search_people(&a, &q)).await
    }

    // -- Served-content ledger (mika#1867) --

    /// Record that Mika has served a piece of content to a specific person
    /// (mika#1867). See [`Database::record_served_content`].
    pub async fn record_served_content(
        &self,
        person_id: i64,
        category: String,
        content_text: String,
        session_id: Option<String>,
    ) -> Result<RecordOutcome> {
        let a = self.agent_id.clone();
        self.with_db(move |db| {
            db.record_served_content(
                &a,
                person_id,
                &category,
                &content_text,
                session_id.as_deref(),
            )
        })
        .await
    }

    /// List served-content rows for `(agent_id, person_id, category)` filtered
    /// by `since` (mika#1867). When `content_hash` is `Some`, pushes the
    /// exact-hash filter into SQL — /ce:review P1-7 fix for the
    /// LIMIT-then-filter false-negative that would let a hash-targeted probe
    /// miss a match sitting below the top-N most-recent window.
    /// See [`Database::list_served_content`].
    pub async fn list_served_content(
        &self,
        person_id: i64,
        category: String,
        since: Option<String>,
        limit: usize,
        content_hash: Option<String>,
    ) -> Result<Vec<ServedContent>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| {
            db.list_served_content(
                &a,
                person_id,
                &category,
                since.as_deref(),
                limit,
                content_hash.as_deref(),
            )
        })
        .await
    }

    // -- Commitments --

    pub async fn add_commitment(
        &self,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        let (a, d, dd) = (
            self.agent_id.clone(),
            description.to_owned(),
            due_date.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.add_commitment(&a, &d, dd.as_deref(), person_id))
            .await
    }

    pub async fn list_commitments(&self, status: &str) -> Result<Vec<Commitment>> {
        let (a, s) = (self.agent_id.clone(), status.to_owned());
        self.with_db(move |db| db.list_commitments(&a, &s)).await
    }

    pub async fn update_commitment_status(&self, id: i64, status: &str) -> Result<bool> {
        let (a, s) = (self.agent_id.clone(), status.to_owned());
        self.with_db(move |db| db.update_commitment_status(&a, id, &s))
            .await
    }

    pub async fn get_commitment_status(&self, id: i64) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_commitment_status(&a, id))
            .await
    }

    pub async fn get_commitment_details(
        &self,
        id: i64,
    ) -> Result<Option<(String, Option<String>)>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_commitment_details(&a, id))
            .await
    }

    pub async fn search_commitments(&self, query: &str) -> Result<Vec<Commitment>> {
        let (a, q) = (self.agent_id.clone(), query.to_owned());
        self.with_db(move |db| db.search_commitments(&a, &q)).await
    }

    // -- Preferences --

    pub async fn set_preference(&self, category: &str, value: &str) -> Result<i64> {
        let (a, c, v) = (self.agent_id.clone(), category.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_preference(&a, &c, &v)).await
    }

    pub async fn get_preference(&self, category: &str) -> Result<Option<String>> {
        let (a, c) = (self.agent_id.clone(), category.to_owned());
        self.with_db(move |db| db.get_preference(&a, &c)).await
    }

    pub async fn list_preferences(&self) -> Result<Vec<Preference>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.list_preferences(&a)).await
    }

    pub async fn search_preferences(&self, query: &str) -> Result<Vec<Preference>> {
        let (a, q) = (self.agent_id.clone(), query.to_owned());
        self.with_db(move |db| db.search_preferences(&a, &q)).await
    }

    // -- Events --

    pub async fn add_event(
        &self,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        let (a, d, ed, c) = (
            self.agent_id.clone(),
            description.to_owned(),
            event_date.map(|s| s.to_owned()),
            context.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.add_event(&a, &d, ed.as_deref(), c.as_deref()))
            .await
    }

    pub async fn list_events(&self) -> Result<Vec<Event>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.list_events(&a)).await
    }

    pub async fn search_events(&self, query: &str) -> Result<Vec<Event>> {
        let (a, q) = (self.agent_id.clone(), query.to_owned());
        self.with_db(move |db| db.search_events(&a, &q)).await
    }

    // -- Housekeeping --

    pub async fn prune_old_heartbeat_sends(&self, days: u32) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.prune_old_heartbeat_sends(&a, days))
            .await
    }

    pub async fn record_heartbeat_send(&self) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.record_heartbeat_send(&a)).await
    }

    pub async fn count_heartbeat_sends_last_hour(&self) -> Result<u32> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_heartbeat_sends_last_hour(&a))
            .await
    }

    pub async fn count_heartbeat_sends_today(&self, timezone: &str) -> Result<u32> {
        let (a, tz) = (self.agent_id.clone(), timezone.to_owned());
        self.with_db(move |db| db.count_heartbeat_sends_today(&a, &tz))
            .await
    }

    pub async fn last_user_message_time(&self) -> Result<Option<String>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.last_user_message_time(&a)).await
    }

    // -- Failed Sends --

    pub async fn save_failed_send(&self, text: &str, request_id: Option<&str>) -> Result<i64> {
        let (a, t, r) = (
            self.agent_id.clone(),
            text.to_owned(),
            request_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.save_failed_send(&a, &t, r.as_deref()))
            .await
    }

    pub async fn get_pending_failed_sends(&self, limit: usize) -> Result<Vec<FailedSend>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_pending_failed_sends(&a, limit))
            .await
    }

    pub async fn delete_failed_send(&self, id: i64) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.delete_failed_send(&a, id)).await
    }

    pub async fn increment_failed_send_retry(&self, id: i64) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.increment_failed_send_retry(&a, id))
            .await
    }

    pub async fn compact_old_audit_events(&self, days: u32) -> Result<usize> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.compact_old_audit_events(&a, days))
            .await
    }

    pub async fn db_size_bytes(&self) -> Result<u64> {
        self.with_db(|db| db.db_size_bytes()).await
    }

    pub async fn schema_version(&self) -> Result<i64> {
        self.with_db(|db| db.schema_version()).await
    }

    pub async fn vacuum(&self) -> Result<()> {
        self.with_db(|db| db.vacuum()).await
    }

    // -- Customer Config --

    pub async fn get_customer_config(&self, key: &str) -> Result<Option<String>> {
        let (a, k) = (self.agent_id.clone(), key.to_owned());
        self.with_db(move |db| db.get_customer_config(&a, &k)).await
    }

    pub async fn set_customer_config(&self, key: &str, value: &str) -> Result<()> {
        let (a, k, v) = (self.agent_id.clone(), key.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_customer_config(&a, &k, &v))
            .await
    }

    pub async fn list_customer_config(&self) -> Result<Vec<(String, String)>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.list_customer_config(&a)).await
    }

    // -- Cross-Session Queries --

    pub async fn load_messages_after(&self, after_id: i64) -> Result<Vec<SessionMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.load_messages_after(&a, after_id))
            .await
    }

    pub async fn max_message_id(&self) -> Result<i64> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.max_message_id(&a)).await
    }

    // -- Audit --

    #[allow(clippy::too_many_arguments)]
    pub async fn log_audit_event(
        &self,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: Option<&str>,
        reasoning: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<()> {
        let (a, sid, tn, tk, bv, av, r, t) = (
            self.agent_id.clone(),
            session_id.to_owned(),
            tool_name.to_owned(),
            target_key.to_owned(),
            before_value.map(|s| s.to_owned()),
            after_value.map(|s| s.to_owned()),
            reasoning.map(|s| s.to_owned()),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.log_audit_event(
                &a,
                &sid,
                &tn,
                &tk,
                bv.as_deref(),
                av.as_deref(),
                r.as_deref(),
                t.as_deref(),
            )
        })
        .await
    }

    pub async fn get_audit_events(&self, session_id: &str) -> Result<Vec<AuditEvent>> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.get_audit_events(&a, &s)).await
    }

    // -- Rewind --

    pub async fn get_audit_events_by_trace_ids(
        &self,
        trace_ids: Vec<String>,
    ) -> Result<Vec<AuditEvent>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_audit_events_by_trace_ids(&a, &trace_ids))
            .await
    }

    pub async fn get_messages_after_id(
        &self,
        session_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.get_messages_after_id(&a, &s, after_id))
            .await
    }

    pub async fn get_compaction_boundary(&self) -> Result<Option<i64>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_compaction_boundary(&a)).await
    }

    pub async fn delete_messages_after_id(&self, session_id: &str, after_id: i64) -> Result<usize> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.delete_messages_after_id(&a, &s, after_id))
            .await
    }

    pub async fn delete_rewind_markers(&self, session_id: &str) -> Result<usize> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.delete_rewind_markers(&a, &s))
            .await
    }

    pub async fn mark_audit_events_rewound(
        &self,
        trace_ids: Vec<String>,
        rewind_trace_id: &str,
    ) -> Result<usize> {
        let (a, rt) = (self.agent_id.clone(), rewind_trace_id.to_owned());
        self.with_db(move |db| db.mark_audit_events_rewound(&a, &trace_ids, &rt))
            .await
    }

    pub async fn delete_person_by_name(&self, name: &str) -> Result<bool> {
        let (a, n) = (self.agent_id.clone(), name.to_owned());
        self.with_db(move |db| db.delete_person_by_name(&a, &n))
            .await
    }

    pub async fn delete_preference(&self, category: &str) -> Result<bool> {
        let (a, c) = (self.agent_id.clone(), category.to_owned());
        self.with_db(move |db| db.delete_preference(&a, &c)).await
    }

    pub async fn delete_commitment_by_description(&self, description: &str) -> Result<bool> {
        let (a, d) = (self.agent_id.clone(), description.to_owned());
        self.with_db(move |db| db.delete_commitment_by_description(&a, &d))
            .await
    }

    pub async fn delete_event_by_description(&self, description: &str) -> Result<bool> {
        let (a, d) = (self.agent_id.clone(), description.to_owned());
        self.with_db(move |db| db.delete_event_by_description(&a, &d))
            .await
    }

    pub async fn get_tasks_by_trace_ids(
        &self,
        trace_ids: &[String],
    ) -> Result<Vec<crate::db::Task>> {
        let (a, t) = (self.agent_id.clone(), trace_ids.to_vec());
        self.with_db(move |db| db.get_tasks_by_trace_ids(&a, &t))
            .await
    }

    pub async fn delete_task_by_id(&self, id: &str) -> Result<bool> {
        let (a, i) = (self.agent_id.clone(), id.to_owned());
        self.with_db(move |db| db.delete_task_by_id(&i, &a)).await
    }

    // -- Reflection --

    pub async fn get_messages_since(&self, since: &str) -> Result<Vec<SessionMessage>> {
        let a = self.agent_id.clone();
        let s = since.to_owned();
        self.with_db(move |db| db.get_messages_since(&a, &s)).await
    }

    pub async fn get_audit_events_since(&self, since: &str) -> Result<Vec<AuditEvent>> {
        let a = self.agent_id.clone();
        let s = since.to_owned();
        self.with_db(move |db| db.get_audit_events_since(&a, &s))
            .await
    }

    pub async fn prune_old_reflection_runs(&self, days: u32) -> Result<usize> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.prune_old_reflection_runs(&a, days))
            .await
    }

    pub async fn record_reflection_run(
        &self,
        status: &str,
        changes_made: i64,
        summary: Option<&str>,
    ) -> Result<()> {
        let (a, st, su) = (
            self.agent_id.clone(),
            status.to_owned(),
            summary.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.record_reflection_run(&a, &st, changes_made, su.as_deref()))
            .await
    }

    pub async fn last_reflection_run_today(&self, timezone: &str) -> Result<bool> {
        let (a, tz) = (self.agent_id.clone(), timezone.to_owned());
        self.with_db(move |db| db.last_reflection_run_today(&a, &tz))
            .await
    }

    pub async fn count_audit_events_for_session(&self, session_id: &str) -> Result<i64> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.count_audit_events_for_session(&a, &s))
            .await
    }

    pub async fn count_core_memory_edits_latest_session(&self) -> Result<i64> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_core_memory_edits_latest_session(&a))
            .await
    }

    // -- Layer 3: Search Indexing --

    pub async fn index_content(
        &self,
        source_type: &str,
        source_id: Option<i64>,
        content: &str,
    ) -> Result<i64> {
        let (a, st, c) = (
            self.agent_id.clone(),
            source_type.to_owned(),
            content.to_owned(),
        );
        self.with_db(move |db| db.index_content(&a, &st, source_id, &c))
            .await
    }

    pub async fn index_embedding(&self, content_id: i64, embedding: Vec<f32>) -> Result<()> {
        self.with_db(move |db| db.index_embedding(content_id, &embedding))
            .await
    }

    pub async fn delete_search_content(&self, source_type: &str, source_id: i64) -> Result<()> {
        let (a, st) = (self.agent_id.clone(), source_type.to_owned());
        self.with_db(move |db| db.delete_search_content(&a, &st, source_id))
            .await
    }

    pub async fn get_unembedded_content(&self) -> Result<Vec<(i64, String)>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_unembedded_content(&a)).await
    }

    pub async fn count_search_content(&self) -> Result<i64> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.count_search_content(&a)).await
    }

    pub async fn fts_search(
        &self,
        query: &str,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let (a, q, st) = (
            self.agent_id.clone(),
            query.to_owned(),
            source_type_filter.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.fts_search(&a, &q, limit, st.as_deref()))
            .await
    }

    pub async fn hybrid_search(
        &self,
        fts_query: &str,
        embedding: Option<Vec<f32>>,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let (a, q, st) = (
            self.agent_id.clone(),
            fts_query.to_owned(),
            source_type_filter.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.hybrid_search(&a, &q, embedding.as_deref(), limit, st.as_deref()))
            .await
    }

    pub async fn get_all_facts_for_indexing(&self) -> Result<Vec<(String, i64, String)>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_all_facts_for_indexing(&a))
            .await
    }

    pub async fn list_facts_paginated_with_count(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<crate::db::DashboardFact>, u64)> {
        let aid = self.agent_id.clone();
        self.with_db(move |db| db.list_facts_paginated_with_count(&aid, limit, offset))
            .await
    }

    // -- Team Runs --

    pub async fn insert_team_run(
        &self,
        run_id: &str,
        team_name: &str,
        goal: &str,
        max_iterations: u32,
        started_at: &str,
        trace_id: Option<&str>,
    ) -> Result<()> {
        let (ri, tn, g, sa, ti) = (
            run_id.to_owned(),
            team_name.to_owned(),
            goal.to_owned(),
            started_at.to_owned(),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.insert_team_run(&ri, &tn, &g, max_iterations, &sa, ti.as_deref()))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_team_run(
        &self,
        run_id: &str,
        status: &str,
        failure_reason: Option<&str>,
        iteration: u32,
        deliverable: Option<&str>,
        ended_at: Option<&str>,
        delegation_count: u32,
        solo_absorption: bool,
        failure_context: Option<&str>,
    ) -> Result<()> {
        let (ri, s, fr, d, ea, fc) = (
            run_id.to_owned(),
            status.to_owned(),
            failure_reason.map(|s| s.to_owned()),
            deliverable.map(|s| s.to_owned()),
            ended_at.map(|s| s.to_owned()),
            failure_context.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.update_team_run(
                &ri,
                &s,
                fr.as_deref(),
                iteration,
                d.as_deref(),
                ea.as_deref(),
                delegation_count,
                solo_absorption,
                fc.as_deref(),
            )
        })
        .await
    }

    /// Find team runs stuck in `status='running'` with no recent child-session
    /// liveness (mika#1652). Not agent-scoped — `team_runs` is shared across the
    /// single container DB.
    pub async fn find_stuck_team_runs(
        &self,
        threshold_secs: i64,
        liveness_threshold_secs: i64,
    ) -> Result<Vec<TeamRunRow>> {
        self.with_db(move |db| db.find_stuck_team_runs(threshold_secs, liveness_threshold_secs))
            .await
    }

    /// Idempotently transition a team run to a terminal status (mika#1652).
    /// Returns `true` when a row changed (was still `running`).
    pub async fn transition_team_run_terminal(
        &self,
        team_run_id: &str,
        status: &str,
        failure_reason: &str,
    ) -> Result<bool> {
        let (ri, s, fr) = (
            team_run_id.to_owned(),
            status.to_owned(),
            failure_reason.to_owned(),
        );
        self.with_db(move |db| db.transition_team_run_terminal(&ri, &s, &fr))
            .await
    }

    pub async fn suspend_team_run(&self, run_id: &str, checkpoint: &str) -> Result<()> {
        let (r, c) = (run_id.to_owned(), checkpoint.to_owned());
        self.with_db(move |db| db.suspend_team_run(&r, &c)).await
    }

    pub async fn load_team_run_checkpoint(&self, run_id: &str) -> Result<Option<String>> {
        let r = run_id.to_owned();
        self.with_db(move |db| db.load_team_run_checkpoint(&r))
            .await
    }

    pub async fn resume_team_run_status(&self, run_id: &str) -> Result<()> {
        let r = run_id.to_owned();
        self.with_db(move |db| db.resume_team_run_status(&r)).await
    }

    pub async fn load_team_run_trace_id(&self, run_id: &str) -> Result<Option<String>> {
        let r = run_id.to_owned();
        self.with_db(move |db| db.load_team_run_trace_id(&r)).await
    }

    pub async fn load_team_runs(&self, team_name: &str, limit: usize) -> Result<Vec<TeamRunRow>> {
        let tn = team_name.to_owned();
        self.with_db(move |db| db.load_team_runs(&tn, limit)).await
    }

    pub async fn load_team_runs_for_prompt(
        &self,
        team_name: &str,
        limit: usize,
        max_text_len: usize,
    ) -> Result<Vec<TeamRunRow>> {
        let tn = team_name.to_owned();
        self.with_db(move |db| db.load_team_runs_for_prompt(&tn, limit, max_text_len))
            .await
    }

    pub async fn load_latest_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let tn = team_name.to_owned();
        self.with_db(move |db| db.load_latest_team_run(&tn)).await
    }

    pub async fn load_team_run_by_id(&self, run_id: &str) -> Result<Option<TeamRunRow>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.load_team_run_by_id(&ri)).await
    }

    pub async fn get_last_completed_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let tn = team_name.to_owned();
        self.with_db(move |db| db.get_last_completed_team_run(&tn))
            .await
    }

    pub async fn get_team_run_summary(&self, run_id: &str) -> Result<Option<TeamRunSummary>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.get_team_run_summary(&ri)).await
    }

    pub async fn get_last_completed_team_run_summary(
        &self,
        team_name: &str,
    ) -> Result<Option<TeamRunSummary>> {
        let tn = team_name.to_owned();
        self.with_db(move |db| db.get_last_completed_team_run_summary(&tn))
            .await
    }

    // -- Team Workspace --

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_team_workspace_entry(
        &self,
        run_id: &str,
        parent_id: Option<i64>,
        agent_name: Option<&str>,
        entry_type: &str,
        content: &str,
        iteration: u32,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        let (ri, an, et, c, t) = (
            run_id.to_owned(),
            agent_name.map(|s| s.to_owned()),
            entry_type.to_owned(),
            content.to_owned(),
            trace_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.insert_team_workspace_entry(
                &ri,
                parent_id,
                an.as_deref(),
                &et,
                &c,
                iteration,
                t.as_deref(),
            )
        })
        .await
    }

    pub async fn load_assignment_entry_ids(
        &self,
        run_id: &str,
        iteration: u32,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.load_assignment_entry_ids(&ri, iteration))
            .await
    }

    pub async fn load_team_workspace(&self, run_id: &str) -> Result<Vec<TeamWorkspaceEntry>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.load_team_workspace(&ri)).await
    }

    // -- Dashboard queries (cross-agent) --

    pub async fn query_timeline(
        &self,
        filters: TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TimelineRow>> {
        self.with_db(move |db| db.query_timeline(&filters, limit, offset))
            .await
    }

    pub async fn query_timeline_count(&self, filters: TimelineFilters) -> Result<u64> {
        self.with_db(move |db| db.query_timeline_count(&filters))
            .await
    }

    pub async fn query_timeline_by_trace(&self, trace_id: &str) -> Result<Vec<TimelineRow>> {
        let tid = trace_id.to_owned();
        self.with_db(move |db| db.query_timeline_by_trace(&tid))
            .await
    }

    pub async fn get_messages_by_trace_id(&self, trace_id: &str) -> Result<Vec<SessionMessage>> {
        let tid = trace_id.to_owned();
        self.with_db(move |db| db.get_messages_by_trace_id(&tid))
            .await
    }

    pub async fn list_agents_with_stats(&self) -> Result<Vec<AgentWithStats>> {
        self.with_db(|db| db.list_agents_with_stats()).await
    }

    pub async fn get_agent_with_stats(&self, agent_id: &str) -> Result<Option<AgentWithStats>> {
        let aid = agent_id.to_owned();
        self.with_db(move |db| db.get_agent_with_stats(&aid)).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_sessions_paginated(
        &self,
        agent_id: Option<String>,
        channel_type: Option<String>,
        session_id: Option<String>,
        task_id: Option<String>,
        from: Option<String>,
        to: Option<String>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionWithStats>> {
        self.with_db(move |db| {
            db.list_sessions_paginated(
                agent_id.as_deref(),
                channel_type.as_deref(),
                session_id.as_deref(),
                task_id.as_deref(),
                from.as_deref(),
                to.as_deref(),
                limit,
                offset,
            )
        })
        .await
    }

    pub async fn count_sessions(
        &self,
        agent_id: Option<String>,
        channel_type: Option<String>,
        session_id: Option<String>,
        task_id: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<u64> {
        self.with_db(move |db| {
            db.count_sessions(
                agent_id.as_deref(),
                channel_type.as_deref(),
                session_id.as_deref(),
                task_id.as_deref(),
                from.as_deref(),
                to.as_deref(),
            )
        })
        .await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let sid = session_id.to_owned();
        self.with_db(move |db| db.get_session(&sid)).await
    }

    /// Get the most recent ended CLI session for an agent.
    pub async fn get_last_cli_session_for_agent(&self, agent_id: &str) -> Result<Option<Session>> {
        let aid = agent_id.to_owned();
        self.with_db(move |db| db.get_last_cli_session_for_agent(&aid))
            .await
    }

    /// Get all sessions linked to a task tree (root task + direct children).
    pub async fn get_sessions_for_task_tree(
        &self,
        root_task_id: &str,
    ) -> Result<Vec<TaskSessionRow>> {
        let tid = root_task_id.to_owned();
        self.with_db(move |db| db.get_sessions_for_task_tree(&tid))
            .await
    }

    pub async fn load_session_messages_paginated(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sid = session_id.to_owned();
        self.with_db(move |db| db.load_session_messages_paginated(&sid, limit, offset))
            .await
    }

    pub async fn count_session_messages(&self, session_id: &str) -> Result<u64> {
        let sid = session_id.to_owned();
        self.with_db(move |db| db.count_session_messages(&sid))
            .await
    }

    pub async fn get_message_by_id(&self, message_id: i64) -> Result<Option<SessionMessage>> {
        self.with_db(move |db| db.get_message_by_id(message_id))
            .await
    }

    pub async fn get_surrounding_messages(
        &self,
        session_id: &str,
        target_id: i64,
        before: u32,
        after: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sid = session_id.to_owned();
        self.with_db(move |db| db.get_surrounding_messages(&sid, target_id, before, after))
            .await
    }

    pub async fn list_audit_events_paginated(
        &self,
        agent_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>> {
        let aid = agent_id.to_owned();
        self.with_db(move |db| db.list_audit_events_paginated(&aid, None, None, limit, offset))
            .await
    }

    pub async fn count_audit_events(&self, agent_id: &str) -> Result<u64> {
        let aid = agent_id.to_owned();
        self.with_db(move |db| db.count_audit_events(&aid, None, None))
            .await
    }

    // -- Combined data+count queries (single channel round-trip) --

    pub async fn query_timeline_with_count(
        &self,
        filters: TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TimelineRow>, u64)> {
        self.with_db(move |db| db.query_timeline_with_count(&filters, limit, offset))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_sessions_paginated_with_count(
        &self,
        agent_id: Option<String>,
        channel_type: Option<String>,
        session_id: Option<String>,
        task_id: Option<String>,
        from: Option<String>,
        to: Option<String>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionWithStats>, u64)> {
        self.with_db(move |db| {
            db.list_sessions_paginated_with_count(
                agent_id.as_deref(),
                channel_type.as_deref(),
                session_id.as_deref(),
                task_id.as_deref(),
                from.as_deref(),
                to.as_deref(),
                limit,
                offset,
            )
        })
        .await
    }

    pub async fn load_session_messages_paginated_with_count(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionMessage>, u64)> {
        let sid = session_id.to_owned();
        self.with_db(move |db| db.load_session_messages_paginated_with_count(&sid, limit, offset))
            .await
    }

    pub async fn list_audit_events_paginated_with_count(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<AuditEvent>, u64)> {
        let aid = agent_id.to_owned();
        let tn = tool_name.map(|s| s.to_owned());
        let tk = target_key.map(|s| s.to_owned());
        self.with_db(move |db| {
            db.list_audit_events_paginated_with_count(
                &aid,
                tn.as_deref(),
                tk.as_deref(),
                limit,
                offset,
            )
        })
        .await
    }

    // -- Dashboard: Tasks --

    pub async fn get_task_unscoped(&self, id: &str) -> Result<Option<Task>> {
        let id = id.to_owned();
        self.with_db(move |db| db.get_task_unscoped(&id)).await
    }

    pub async fn list_tasks_paginated_with_count(
        &self,
        filters: TaskFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        self.with_db(move |db| db.list_tasks_paginated_with_count(&filters, limit, offset))
            .await
    }

    // -- Dashboard: Team Runs --

    pub async fn list_team_runs_paginated_with_count(
        &self,
        filters: TeamRunFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TeamRunRow>, u64)> {
        self.with_db(move |db| db.list_team_runs_paginated_with_count(&filters, limit, offset))
            .await
    }

    // -- Dashboard: Dev Runs --

    pub async fn update_task_metadata(&self, task_id: &str, metadata_json: &str) -> Result<bool> {
        let (i, m) = (task_id.to_owned(), metadata_json.to_owned());
        self.with_db(move |db| db.update_task_metadata(&i, &m))
            .await
    }

    pub async fn get_dev_run(&self, task_id: &str) -> Result<Option<Task>> {
        let i = task_id.to_owned();
        self.with_db(move |db| db.get_dev_run(&i)).await
    }

    pub async fn list_dev_runs_paginated_with_count(
        &self,
        status: Option<String>,
        from: Option<String>,
        to: Option<String>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        self.with_db(move |db| {
            db.list_dev_runs_paginated_with_count(
                status.as_deref(),
                from.as_deref(),
                to.as_deref(),
                limit,
                offset,
            )
        })
        .await
    }

    // -- A2A Protocol --

    /// Create an A2A task (creates entries in tasks, sessions, and a2a_task_map).
    /// Returns the session_id for use with the agent loop.
    pub async fn a2a_create_task(
        &self,
        a2a_task_id: &str,
        context_id: Option<&str>,
    ) -> Result<String> {
        let (i, a, c) = (
            a2a_task_id.to_owned(),
            self.agent_id.clone(),
            context_id.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.a2a_create_task(&i, &a, c.as_deref()))
            .await
    }

    pub async fn a2a_get_task_state(&self, id: &str) -> Result<Option<String>> {
        let i = id.to_owned();
        self.with_db(move |db| db.a2a_get_task_state(&i)).await
    }

    pub async fn a2a_update_task_state(&self, id: &str, state: &str) -> Result<()> {
        let (i, s) = (id.to_owned(), state.to_owned());
        self.with_db(move |db| db.a2a_update_task_state(&i, &s))
            .await
    }

    pub async fn a2a_insert_message(
        &self,
        a2a_task_id: &str,
        message: &mika_a2a::types::Message,
    ) -> Result<()> {
        let (t, a, m) = (
            a2a_task_id.to_owned(),
            self.agent_id.clone(),
            message.clone(),
        );
        self.with_db(move |db| db.a2a_insert_message(&t, &a, &m))
            .await
    }

    pub async fn a2a_build_task(
        &self,
        id: &str,
        history_length: Option<i32>,
    ) -> Result<Option<mika_a2a::types::Task>> {
        let i = id.to_owned();
        self.with_db(move |db| db.a2a_build_task(&i, history_length))
            .await
    }

    pub async fn a2a_get_session_id(&self, a2a_task_id: &str) -> Result<Option<String>> {
        let i = a2a_task_id.to_owned();
        self.with_db(move |db| db.a2a_get_session_id(&i)).await
    }

    pub async fn a2a_set_push_config(
        &self,
        config: &mika_a2a::types::TaskPushNotificationConfig,
    ) -> Result<()> {
        let c = config.clone();
        self.with_db(move |db| db.a2a_set_push_config(&c)).await
    }

    pub async fn a2a_get_push_config(
        &self,
        id: &str,
    ) -> Result<Option<mika_a2a::types::TaskPushNotificationConfig>> {
        let i = id.to_owned();
        self.with_db(move |db| db.a2a_get_push_config(&i)).await
    }

    pub async fn a2a_list_push_configs(
        &self,
        task_id: &str,
    ) -> Result<Vec<mika_a2a::types::TaskPushNotificationConfig>> {
        let t = task_id.to_owned();
        self.with_db(move |db| db.a2a_list_push_configs(&t)).await
    }

    pub async fn a2a_delete_push_config(&self, id: &str) -> Result<bool> {
        let i = id.to_owned();
        self.with_db(move |db| db.a2a_delete_push_config(&i)).await
    }

    // -- Observability: LLM Calls + Tool Calls --

    #[allow(clippy::too_many_arguments)]
    pub async fn save_llm_call(
        &self,
        id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        latency_ms: u64,
        stop_reason: Option<&str>,
        status: &str,
        error_message: Option<&str>,
        step: u32,
        prompt_variant: Option<&str>,
        response_text: Option<&str>,
        reasoning: Option<&str>,
        system_prompt_bytes: Option<i64>,
    ) -> Result<()> {
        let (a, i, sid, tid, p, m, sr, st, em, pv, rt, rz) = (
            self.agent_id.clone(),
            id.to_owned(),
            session_id.to_owned(),
            trace_id.map(|s| s.to_owned()),
            provider.to_owned(),
            model.to_owned(),
            stop_reason.map(|s| s.to_owned()),
            status.to_owned(),
            error_message.map(|s| s.to_owned()),
            prompt_variant.map(|s| s.to_owned()),
            response_text.map(|s| s.to_owned()),
            reasoning.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.save_llm_call(
                &i,
                &a,
                &sid,
                tid.as_deref(),
                &p,
                &m,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                latency_ms,
                sr.as_deref(),
                &st,
                em.as_deref(),
                step,
                pv.as_deref(),
                rt.as_deref(),
                rz.as_deref(),
                system_prompt_bytes,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_tool_call(
        &self,
        id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        llm_call_id: Option<&str>,
        step: u32,
        tool_name: &str,
        tool_source: &str,
        skill_name: Option<&str>,
        input: Option<&str>,
        output: Option<&str>,
        success: bool,
        non_zero_exit: bool,
        latency_ms: u64,
        error_message: Option<&str>,
    ) -> Result<()> {
        let (a, i, sid, tid, lid, tn, ts, sn, inp, out, em) = (
            self.agent_id.clone(),
            id.to_owned(),
            session_id.to_owned(),
            trace_id.map(|s| s.to_owned()),
            llm_call_id.map(|s| s.to_owned()),
            tool_name.to_owned(),
            tool_source.to_owned(),
            skill_name.map(|s| s.to_owned()),
            input.map(|s| s.to_owned()),
            output.map(|s| s.to_owned()),
            error_message.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.save_tool_call(
                &i,
                &a,
                &sid,
                tid.as_deref(),
                lid.as_deref(),
                step,
                &tn,
                &ts,
                sn.as_deref(),
                inp.as_deref(),
                out.as_deref(),
                success,
                non_zero_exit,
                latency_ms,
                em.as_deref(),
            )
        })
        .await
    }

    pub async fn prune_old_llm_calls(&self, retention_secs: i64) -> Result<usize> {
        self.with_db(move |db| db.prune_old_llm_calls(retention_secs))
            .await
    }

    pub async fn prune_old_tool_calls(&self, retention_secs: i64) -> Result<usize> {
        self.with_db(move |db| db.prune_old_tool_calls(retention_secs))
            .await
    }

    /// Insert a batch of pilot-transcript rows for one task atomically
    /// (mika#1705). Bodies must already be secret-scrubbed by the caller.
    pub async fn insert_pilot_transcripts_batch(
        &self,
        task_id: String,
        rows: Vec<crate::db::PilotTranscriptRow>,
    ) -> Result<usize> {
        self.with_db(move |db| db.insert_pilot_transcripts_batch(&task_id, &rows))
            .await
    }

    /// Prune pilot transcripts older than `retention_secs` (mika#1705 AC6).
    pub async fn prune_old_pilot_transcripts(&self, retention_secs: i64) -> Result<usize> {
        self.with_db(move |db| db.prune_old_pilot_transcripts(retention_secs))
            .await
    }

    /// Count pilot transcripts for a task (mika#1705 idempotency guard).
    pub async fn count_pilot_transcripts_for_task(&self, task_id: String) -> Result<i64> {
        self.with_db(move |db| db.count_pilot_transcripts_for_task(&task_id))
            .await
    }

    pub async fn query_llm_calls_by_trace(
        &self,
        trace_id: &str,
    ) -> Result<Vec<crate::db::LlmCallRow>> {
        let t = trace_id.to_owned();
        self.with_db(move |db| db.query_llm_calls_by_trace(&t))
            .await
    }

    pub async fn query_tool_calls_by_trace(
        &self,
        trace_id: &str,
    ) -> Result<Vec<crate::db::ToolCallRow>> {
        let t = trace_id.to_owned();
        self.with_db(move |db| db.query_tool_calls_by_trace(&t))
            .await
    }

    pub async fn query_llm_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<crate::db::LlmCallRow>, u64)> {
        let s = session_id.to_owned();
        self.with_db(move |db| db.query_llm_calls_by_session(&s, page, per_page))
            .await
    }

    pub async fn query_tool_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<crate::db::ToolCallRow>, u64)> {
        let s = session_id.to_owned();
        self.with_db(move |db| db.query_tool_calls_by_session(&s, page, per_page))
            .await
    }

    pub async fn query_llm_calls(
        &self,
        filters: crate::db::LlmCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<crate::db::LlmCallRow>, u64)> {
        self.with_db(move |db| db.query_llm_calls(&filters, page, per_page))
            .await
    }

    pub async fn query_cost_trend(
        &self,
        filters: crate::db::CostTrendFilters,
    ) -> Result<crate::db::CostTrendResponse> {
        self.with_db(move |db| db.query_cost_trend(&filters)).await
    }

    pub async fn query_tool_calls(
        &self,
        filters: crate::db::ToolCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<crate::db::ToolCallRow>, u64)> {
        self.with_db(move |db| db.query_tool_calls(&filters, page, per_page))
            .await
    }

    pub async fn get_llm_call_by_id(&self, id: &str) -> Result<Option<crate::db::LlmCallRow>> {
        let id = id.to_owned();
        self.with_db(move |db| db.get_llm_call_by_id(&id)).await
    }

    pub async fn get_tool_call_by_id(&self, id: &str) -> Result<Option<crate::db::ToolCallRow>> {
        let id = id.to_owned();
        self.with_db(move |db| db.get_tool_call_by_id(&id)).await
    }

    pub async fn get_tool_calls_by_llm_call_id(
        &self,
        llm_call_id: &str,
    ) -> Result<Vec<crate::db::ToolCallRow>> {
        let id = llm_call_id.to_owned();
        self.with_db(move |db| db.get_tool_calls_by_llm_call_id(&id))
            .await
    }

    pub async fn update_session_metadata(&self, session_id: &str, metadata: &str) -> Result<()> {
        let (sid, m) = (session_id.to_owned(), metadata.to_owned());
        self.with_db(move |db| db.update_session_metadata(&sid, &m))
            .await
    }

    // ── KG corpus queries (#778) ──────────────────────────────────────────

    pub async fn count_chunks_for_docs_root_hash(&self, docs_root_hash: &str) -> Result<u64> {
        let h = docs_root_hash.to_owned();
        self.with_db(move |db| db.count_chunks_for_docs_root_hash(&h))
            .await
    }

    /// Register an agent-corpus mapping (#798). Idempotent.
    pub async fn register_agent_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
        docs_root_path: &str,
    ) -> Result<()> {
        let a = agent_id.to_owned();
        let h = docs_root_hash.to_owned();
        let p = docs_root_path.to_owned();
        self.with_db(move |db| db.register_agent_corpus(&a, &h, &p))
            .await
    }

    /// List all corpora for an agent (#798).
    pub async fn list_agent_corpora(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let a = agent_id.to_owned();
        self.with_db(move |db| db.list_agent_corpora(&a)).await
    }

    // -- Operational: What's Next canonical read path (mika#1263) --

    /// Canonical read-path entry point for the What's Next engine.
    ///
    /// Encapsulates the full score-derive-rank pipeline:
    /// 1. Query non-Done items for the agent.
    /// 2. Batch-load blocked counts (single GROUP BY query).
    /// 3. Run `resolve_cleared_blockers()` to clear stale `blocked_by` references.
    /// 4. Run `derive_status()` on each non-Done item, updating cache if changed.
    /// 5. Run `priority()` scoring on each item using batch-loaded counts.
    /// 6. Write back updated priority scores.
    /// 7. Sort by priority DESC.
    /// 8. Return top `limit` items with breakdowns.
    ///
    /// Feature gate (`MIKA_OPERATIONAL_PARTNER`) is NOT checked here — that's
    /// a surface concern (CLI, dashboard, agent prompt injection). This method
    /// has no knowledge of the feature flag.
    pub async fn score_and_rank_items(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<crate::operational::scoring::ScoredItem>> {
        let aid = agent_id.to_owned();
        self.with_db(move |db| {
            use crate::operational::scoring;
            use crate::operational::status;
            use crate::operational::types::{NonTerminalStatus, OperationalStatus};
            use chrono::Utc;

            let now = Utc::now();

            // 1. Query non-Done items
            let mut items = db.query_active_operational_items(&aid, limit)?;

            // 2. Batch-load blocked counts
            let blocked_counts = db.batch_blocked_counts(&aid)?;

            // 3. Resolve cleared blockers (primary cascade mechanism)
            status::resolve_cleared_blockers(&items, db)?;

            // Re-read items after cascade (blocked_by may have changed)
            items = db.query_active_operational_items(&aid, limit)?;

            // 4. Re-derive status and update cache
            for item in &items {
                let derived = status::derive_status(item, now);
                if derived != item.status {
                    // Convert to NonTerminalStatus for the update method
                    let non_terminal = match derived {
                        OperationalStatus::Now => NonTerminalStatus::Now,
                        OperationalStatus::Waiting => NonTerminalStatus::Waiting,
                        OperationalStatus::Delegated => NonTerminalStatus::Delegated,
                        OperationalStatus::Scheduled => NonTerminalStatus::Scheduled,
                        OperationalStatus::AtRisk => NonTerminalStatus::AtRisk,
                        OperationalStatus::Done => continue, // should not happen for active items
                    };
                    db.update_operational_item_status(&item.id, non_terminal)?;
                }
            }

            // Re-read after status updates (status changes affect presentation)
            let items = db.query_active_operational_items(&aid, limit)?;

            // 5-7. Score, rank, and write back priorities
            let ranked = scoring::rank(items, now, &blocked_counts);

            // 6. Write back priority scores
            for scored in &ranked {
                db.update_operational_item_priority(&scored.item.id, scored.breakdown.total)?;
            }

            // 8. Return top `limit` items
            Ok(ranked)
        })
        .await
    }

    /// Async wrapper for [`Database::insert_permission_decision`] (mika#1733
    /// AC4). Fire-and-forget from the classifier's perspective — the caller
    /// spawns this on a `tokio::spawn` so a slow DB never blocks the
    /// oneshot-first decision path.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_permission_decision(
        &self,
        id: String,
        request_id: String,
        tool_name: String,
        args_summary: Option<String>,
        classifier_verdict: String,
        operator_decision: Option<String>,
        override_used: bool,
        decision_authority: String,
        tenant_id: Option<String>,
        agent_id: Option<String>,
    ) -> Result<()> {
        self.with_db(move |db| {
            db.insert_permission_decision(
                &id,
                &request_id,
                &tool_name,
                args_summary.as_deref(),
                &classifier_verdict,
                operator_decision.as_deref(),
                override_used,
                &decision_authority,
                tenant_id.as_deref(),
                agent_id.as_deref(),
            )
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn test_async_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        AsyncDatabase::new(db)
    }

    async fn test_async_db_with_session() -> (AsyncDatabase, String) {
        let db = test_async_db();
        let sid = "test-session".to_string();
        db.create_session(&sid, "mika", "cli").await.unwrap();
        (db, sid)
    }

    #[tokio::test]
    async fn test_async_save_and_load() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "Hello!", None).await.unwrap();
        db.save_message(&sid, "assistant", "Hi there!", None)
            .await
            .unwrap();
        let messages = db.load_recent_messages(10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_async_concurrent_reads() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "Message 1", None)
            .await
            .unwrap();
        db.save_message(&sid, "user", "Message 2", None)
            .await
            .unwrap();
        let mut handles = Vec::new();
        for _ in 0..5 {
            let db_clone = db.clone();
            handles.push(tokio::spawn(async move {
                db_clone.load_recent_messages(10).await.unwrap()
            }));
        }
        for handle in handles {
            let messages = handle.await.unwrap();
            assert_eq!(messages.len(), 2);
        }
    }

    #[tokio::test]
    async fn test_async_clone_shares_connection() {
        let (db, sid) = test_async_db_with_session().await;
        let db2 = db.clone();
        db.save_message(&sid, "user", "From clone 1", None)
            .await
            .unwrap();
        let messages = db2.load_recent_messages(10).await.unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_async_db_survives_panic() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "before panic", None)
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let sender = db.inner.sender.lock().unwrap().as_ref().unwrap().clone();
        sender
            .send(Box::new(move |_db| {
                let _ = tx.send(());
                panic!("intentional test panic");
            }))
            .await
            .unwrap();
        rx.await.unwrap();
        let messages = db.load_recent_messages(10).await.unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_async_core_memory_roundtrip() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let entries = db.get_all_core_memory().await.unwrap();
        assert!(!entries.is_empty());
        let entry = db.get_core_memory("user_summary").await.unwrap().unwrap();
        assert_eq!(entry.value, "No information about the user yet.");
    }

    #[tokio::test]
    async fn test_shutdown_joins_thread() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "before shutdown", None)
            .await
            .unwrap();
        let messages = db.load_recent_messages(10).await.unwrap();
        assert_eq!(messages.len(), 1);
        db.shutdown();
    }

    #[tokio::test]
    async fn test_shutdown_rejects_subsequent_operations() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "msg", None).await.unwrap();
        db.shutdown();
        let result = db.load_recent_messages(10).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shut down"));
    }

    #[tokio::test]
    async fn test_shutdown_idempotent() {
        let (db, sid) = test_async_db_with_session().await;
        db.save_message(&sid, "user", "msg", None).await.unwrap();
        db.shutdown();
        db.shutdown();
    }

    #[tokio::test]
    async fn test_last_user_message_time_returns_string() {
        let (db, sid) = test_async_db_with_session().await;
        assert!(db.last_user_message_time().await.unwrap().is_none());
        db.save_message(&sid, "user", "hello", None).await.unwrap();
        let ts = db.last_user_message_time().await.unwrap();
        assert!(ts.is_some());
        // ISO 8601 timestamps start with a year like "2026-"
        assert!(ts.unwrap().starts_with("20"));
    }

    #[tokio::test]
    async fn test_with_agent_scoping() {
        let (db, sid) = test_async_db_with_session().await;
        // Register a second agent and create a session for it
        db.register_agent("agent2", "Agent 2", "").await.unwrap();
        let db2 = db.with_agent("agent2");
        db2.create_session("agent2-session", "agent2", "cli")
            .await
            .unwrap();
        db.save_message(&sid, "user", "from main", None)
            .await
            .unwrap();
        db2.save_message("agent2-session", "user", "from agent2", None)
            .await
            .unwrap();
        let main_msgs = db.load_recent_messages(10).await.unwrap();
        let agent2_msgs = db2.load_recent_messages(10).await.unwrap();
        assert_eq!(main_msgs.len(), 1);
        assert_eq!(agent2_msgs.len(), 1);
        assert_eq!(main_msgs[0].content, "from main");
        assert_eq!(agent2_msgs[0].content, "from agent2");
    }

    #[tokio::test]
    async fn test_create_and_get_task() {
        let db = test_async_db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Async reminder".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
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
        let t = db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(t.label, "Async reminder");
        assert_eq!(t.status, "pending");
    }

    #[tokio::test]
    async fn test_save_message_as_uses_explicit_agent_id() {
        let (db, _sid) = test_async_db_with_session().await;

        // Register a second agent and create a team session
        db.register_agent("researcher", "Researcher", "")
            .await
            .unwrap();
        let team_sid = "team-run-123";
        // Create session owned by the orchestrator (mika)
        db.create_session(team_sid, "mika", "team").await.unwrap();

        // save_message_with_metadata uses self.agent_id (default "mika")
        db.save_message_with_metadata(team_sid, "assistant", "from default", None, None, false)
            .await
            .unwrap();

        // save_message_as uses explicit agent_id
        db.save_message_as(
            "researcher",
            team_sid,
            "assistant",
            "from researcher",
            None,
            None,
        )
        .await
        .unwrap();

        // Verify: load all messages for the session and check agent_ids
        let msgs = db
            .with_db({
                let sid = team_sid.to_owned();
                move |db_inner| db_inner.load_session_messages_paginated(&sid, 10, 0)
            })
            .await
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].agent_id, "mika");
        assert_eq!(msgs[0].content, "from default");
        assert_eq!(msgs[1].agent_id, "researcher");
        assert_eq!(msgs[1].content, "from researcher");
    }

    #[tokio::test]
    async fn test_save_message_as_with_metadata() {
        let (db, _sid) = test_async_db_with_session().await;

        db.register_agent("writer", "Writer", "").await.unwrap();
        let team_sid = "team-run-456";
        db.create_session(team_sid, "mika", "team").await.unwrap();

        let metadata = r#"{"agent_name":"writer","team_run_id":"run-456"}"#;
        let trace = "aabbccdd00000000aabbccdd00000000";
        db.save_message_as(
            "writer",
            team_sid,
            "assistant",
            "deliverable content",
            Some(metadata),
            Some(trace),
        )
        .await
        .unwrap();

        let msgs = db
            .with_db({
                let sid = team_sid.to_owned();
                move |db_inner| db_inner.load_session_messages_paginated(&sid, 10, 0)
            })
            .await
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].agent_id, "writer");
        assert_eq!(msgs[0].content, "deliverable content");
        assert!(msgs[0].metadata.as_ref().unwrap().contains("writer"));
    }

    /// Integration test for the full `score_and_rank_items()` 8-step pipeline (AC7).
    ///
    /// Exercises: query non-Done items → batch blocked counts → resolve cleared
    /// blockers → status re-derivation → scoring → priority write-back → sort →
    /// return top N.
    #[tokio::test]
    async fn test_score_and_rank_items_pipeline() {
        use crate::operational::types::{
            EvidenceRef, EvidenceRefKind, NewOperationalItem, OperationalKind, OperationalStatus,
            Owner,
        };

        let db = test_async_db();
        let agent = "test-agent";

        // --- Insert 5 items with varied scoring properties ---

        // Item A: User commitment, high importance, near deadline → should score highest
        let id_a = db
            .with_db({
                let agent = agent.to_string();
                move |db_inner| {
                    let item = NewOperationalItem {
                        kind: OperationalKind::Commitment,
                        title: "Urgent user commitment".to_string(),
                        status: OperationalStatus::Now,
                        owner: Owner::User,
                        priority: 0.0,
                        user_importance: 40.0,
                        due_at: Some(
                            (chrono::Utc::now() + chrono::Duration::hours(2))
                                .format("%Y-%m-%dT%H:%M:%SZ")
                                .to_string(),
                        ),
                        blocked_by: None,
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 1.0,
                        source_table: None,
                        source_id: None,
                        agent_id: agent,
                    };
                    // Stale: set updated_at 10 hours ago to get stale_time contribution
                    let id = db_inner.insert_operational_item(&item)?;
                    let stale_ts = (chrono::Utc::now() - chrono::Duration::hours(10))
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string();
                    db_inner.conn.execute(
                        "UPDATE operational_items SET updated_at = ?1 WHERE id = ?2",
                        rusqlite::params![stale_ts, id],
                    )?;
                    Ok::<String, anyhow::Error>(id)
                }
            })
            .await
            .unwrap();

        // Item B: Regular task, medium importance, low confidence → moderate score with penalty
        let id_b = db
            .with_db({
                let agent = agent.to_string();
                move |db_inner| {
                    let item = NewOperationalItem {
                        kind: OperationalKind::Task,
                        title: "Low confidence task".to_string(),
                        status: OperationalStatus::Now,
                        owner: Owner::User,
                        priority: 0.0,
                        user_importance: 20.0,
                        due_at: None,
                        blocked_by: None,
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 0.3, // 70% penalty
                        source_table: None,
                        source_id: None,
                        agent_id: agent,
                    };
                    db_inner.insert_operational_item(&item)
                }
            })
            .await
            .unwrap();

        // Item C: Blocker that blocks other items → gets dependency_risk bonus
        let id_c = db
            .with_db({
                let agent = agent.to_string();
                move |db_inner| {
                    db_inner.insert_operational_item(&NewOperationalItem {
                        kind: OperationalKind::Blocker,
                        title: "Critical blocker".to_string(),
                        status: OperationalStatus::Now,
                        owner: Owner::User,
                        priority: 0.0,
                        user_importance: 10.0,
                        due_at: None,
                        blocked_by: None,
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 1.0,
                        source_table: None,
                        source_id: None,
                        agent_id: agent,
                    })
                }
            })
            .await
            .unwrap();

        // Item D: Waiting on item C (blocker) → should derive as Waiting
        let id_d = db
            .with_db({
                let agent = agent.to_string();
                let blocked_by = id_c.clone();
                move |db_inner| {
                    db_inner.insert_operational_item(&NewOperationalItem {
                        kind: OperationalKind::Task,
                        title: "Blocked task".to_string(),
                        status: OperationalStatus::Now, // inserted as Now, should re-derive to Waiting
                        owner: Owner::User,
                        priority: 0.0,
                        user_importance: 30.0,
                        due_at: None,
                        blocked_by: Some(blocked_by),
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 1.0,
                        source_table: None,
                        source_id: None,
                        agent_id: agent,
                    })
                }
            })
            .await
            .unwrap();

        // Item E: Mika-owned commitment (lower weight than User commitment).
        // Inserted as Scheduled so derive_status() can re-derive to Delegated
        // (Now has higher precedence than Delegated, so inserting as Now would
        // preserve Now via the explicit-Now rule in is_now()).
        let id_e = db
            .with_db({
                let agent = agent.to_string();
                move |db_inner| {
                    db_inner.insert_operational_item(&NewOperationalItem {
                        kind: OperationalKind::Commitment,
                        title: "Mika commitment".to_string(),
                        status: OperationalStatus::Scheduled,
                        owner: Owner::Mika,
                        priority: 0.0,
                        user_importance: 15.0,
                        due_at: None,
                        blocked_by: None,
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 1.0,
                        source_table: Some("tasks".to_string()),
                        source_id: Some("task-100".to_string()),
                        agent_id: agent,
                    })
                }
            })
            .await
            .unwrap();

        // Item F: Done item — should NOT appear in results
        let _id_f = db
            .with_db({
                let agent = agent.to_string();
                move |db_inner| {
                    let id = db_inner.insert_operational_item(&NewOperationalItem {
                        kind: OperationalKind::Task,
                        title: "Completed task".to_string(),
                        status: OperationalStatus::Now,
                        owner: Owner::User,
                        priority: 100.0,
                        user_importance: 50.0,
                        due_at: None,
                        blocked_by: None,
                        next_action: None,
                        evidence_refs: Vec::new(),
                        confidence: 1.0,
                        source_table: None,
                        source_id: None,
                        agent_id: agent,
                    })?;
                    db_inner.complete_operational_item(
                        &id,
                        EvidenceRef {
                            kind: EvidenceRefKind::External,
                            id: "test-completion".to_string(),
                        },
                    )?;
                    Ok::<String, anyhow::Error>(id)
                }
            })
            .await
            .unwrap();

        // --- Execute the pipeline ---
        let results = db.score_and_rank_items(agent, 50).await.unwrap();

        // --- Assertions ---

        // 1. Done item (F) excluded
        assert_eq!(results.len(), 5, "should return 5 non-Done items");
        assert!(
            !results.iter().any(|s| s.item.title == "Completed task"),
            "Done items must be excluded"
        );

        // 2. All items have non-zero or computed priorities written back
        // (item A should have a high score due to urgency + commitment + importance + stale)
        let item_a = results.iter().find(|s| s.item.id == id_a).unwrap();
        assert!(
            item_a.breakdown.total > 0.0,
            "Item A should have a positive score"
        );
        assert!(
            item_a.breakdown.urgency > 0.0,
            "Item A has a near deadline — urgency should be positive"
        );
        assert!(
            item_a.breakdown.commitment_weight > 0.0,
            "Item A is a User commitment — weight should be positive"
        );
        assert!(
            item_a.breakdown.stale_time > 0.0,
            "Item A was set stale — stale_time should be positive"
        );
        assert!(
            (item_a.breakdown.confidence_penalty - 0.0).abs() < f32::EPSILON,
            "Item A has confidence=1.0 — no penalty"
        );

        // 3. Confidence penalty applied to item B
        let item_b = results.iter().find(|s| s.item.id == id_b).unwrap();
        assert!(
            item_b.breakdown.confidence_penalty > 30.0,
            "Item B confidence=0.3 → penalty = 0.7 * 50 = 35"
        );

        // 4. Item C (blocker) gets dependency_risk from item D blocking on it
        let item_c = results.iter().find(|s| s.item.id == id_c).unwrap();
        assert!(
            item_c.breakdown.dependency_risk > 0.0,
            "Item C has an item blocked on it — dependency_risk should be positive"
        );

        // 5. Status re-derivation: Item D should now be Waiting (blocked_by is set)
        let item_d = results.iter().find(|s| s.item.id == id_d).unwrap();
        assert_eq!(
            item_d.item.status,
            OperationalStatus::Waiting,
            "Item D has blocked_by set — status should be re-derived to Waiting"
        );

        // 6. Item E (Mika-owned with source_table) should be Delegated after re-derivation
        let item_e = results.iter().find(|s| s.item.id == id_e).unwrap();
        assert_eq!(
            item_e.item.status,
            OperationalStatus::Delegated,
            "Item E is Mika-owned with source_table — should re-derive to Delegated"
        );
        // Mika commitment weight = 35.0
        assert!(
            (item_e.breakdown.commitment_weight - 35.0).abs() < f32::EPSILON,
            "Item E is a Mika commitment — weight should be 35.0"
        );

        // 7. Ordering: Item A should be first (highest score)
        assert_eq!(
            results[0].item.id, id_a,
            "Item A (urgent user commitment + high importance + stale) should rank first"
        );

        // 8. Results are sorted DESC by total
        for pair in results.windows(2) {
            assert!(
                pair[0].breakdown.total >= pair[1].breakdown.total,
                "Results must be sorted by priority DESC: {} >= {}",
                pair[0].breakdown.total,
                pair[1].breakdown.total,
            );
        }

        // 9. Priority write-back: verify the DB column matches the computed score
        let written_priorities = db
            .with_db({
                let ids: Vec<String> = results.iter().map(|s| s.item.id.clone()).collect();
                move |db_inner| {
                    let mut out = Vec::new();
                    for id in &ids {
                        let p: f32 = db_inner.conn.query_row(
                            "SELECT priority FROM operational_items WHERE id = ?1",
                            rusqlite::params![id],
                            |row| row.get(0),
                        )?;
                        out.push((id.clone(), p));
                    }
                    Ok::<Vec<(String, f32)>, anyhow::Error>(out)
                }
            })
            .await
            .unwrap();

        for (id, db_priority) in &written_priorities {
            let scored = results.iter().find(|s| &s.item.id == id).unwrap();
            assert!(
                (db_priority - scored.breakdown.total).abs() < f32::EPSILON,
                "Priority write-back mismatch for {id}: DB={db_priority}, computed={}",
                scored.breakdown.total,
            );
        }

        // 10. Status write-back: verify the DB status column matches re-derived status
        let db_status_d: String = db
            .with_db({
                let id = id_d.clone();
                move |db_inner| {
                    db_inner
                        .conn
                        .query_row(
                            "SELECT status FROM operational_items WHERE id = ?1",
                            rusqlite::params![id],
                            |row| row.get(0),
                        )
                        .map_err(Into::into)
                }
            })
            .await
            .unwrap();
        assert_eq!(
            db_status_d, "waiting",
            "DB status for blocked item D should be updated to 'waiting'"
        );
    }

    /// Regression test for mika#1258: under channel saturation, the Tokio worker
    /// pool must remain responsive (async backpressure via tokio::sync::mpsc).
    ///
    /// Strategy: spawn many slow DB closures (each sleeps 100ms) to fill the
    /// channel, then verify a concurrent "control" task continues incrementing
    /// a counter without being blocked.
    #[tokio::test]
    async fn test_async_db_saturated_channel_does_not_pin_workers() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;

        let db = test_async_db();
        let control_counter = Arc::new(AtomicU32::new(0));

        // Spawn a control task that increments a counter every 10ms.
        // If Tokio workers are pinned, this task won't get scheduled.
        let counter_clone = Arc::clone(&control_counter);
        let control_handle = tokio::spawn(async move {
            for _ in 0..200 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                counter_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        // Flood the channel with slow closures that simulate DB saturation.
        // 50 closures × 100ms sleep = enough to keep the worker thread busy
        // while the channel fills up (channel capacity = 512).
        let mut handles = Vec::new();
        for _ in 0..50 {
            let db_clone = db.clone();
            handles.push(tokio::spawn(async move {
                let _ = db_clone
                    .with_db(|_db| {
                        std::thread::sleep(Duration::from_millis(100));
                        Ok(())
                    })
                    .await;
            }));
        }

        // Wait for the control task to finish (should take ~2s).
        control_handle.await.unwrap();

        // The control counter should have incremented substantially.
        // With async backpressure, the Tokio worker pool stays free to
        // schedule the control task. With the old blocking send, workers
        // would pin on send() and the counter would stall.
        let final_count = control_counter.load(Ordering::Relaxed);
        assert!(
            final_count >= 100,
            "control task only ran {final_count}/200 times — \
             Tokio workers may be pinned by blocking channel send"
        );

        // Clean up: wait for all DB tasks to finish.
        for h in handles {
            let _ = h.await;
        }
        db.shutdown();
    }
}
