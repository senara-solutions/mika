use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    mpsc::{self, SyncSender},
};
use std::thread::JoinHandle;
use tokio::sync::oneshot;

use crate::db::{
    AgentRow, Commitment, ConversationMessage, CoreMemoryEntry, Database, Event, FailedSend,
    MemoryEvent, NewTask, Person, Preference, SearchResult, Task, TeamMessageRow, TeamRow,
    TeamRunRow,
};

type DbClosure = Box<dyn FnOnce(&Database) + Send>;

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
    /// The agent context for all operations on this handle (default: "main").
    pub agent_id: String,
}

struct AsyncDatabaseInner {
    sender: Mutex<Option<SyncSender<DbClosure>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}

impl AsyncDatabase {
    /// Spawn a dedicated OS thread that owns `db` and processes closures.
    pub fn new(db: Database) -> Self {
        Self::new_with_agent(db, "main")
    }

    /// Spawn with a specific agent_id.
    pub fn new_with_agent(db: Database, agent_id: &str) -> Self {
        // Bounded channel: backpressure when DB thread falls behind
        let (tx, rx) = mpsc::sync_channel::<DbClosure>(512);
        let handle = std::thread::Builder::new()
            .name("mika-db".to_string())
            .spawn(move || {
                while let Ok(f) = rx.recv() {
                    if let Err(_panic) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            f(&db);
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
            }),
            agent_id: agent_id.to_string(),
        }
    }

    /// Open a database at `path` and wrap it in an async handle (agent_id = "main").
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

    async fn with_db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Database) -> Result<T> + Send + 'static,
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
        self.with_db(move |db| db.create_task(&task)).await
    }

    pub async fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        self.with_db(move |db| db.create_recurring_task_if_absent(task))
            .await
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_task(&i, &a)).await
    }

    pub async fn get_schedulable_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_schedulable_tasks(&id)).await
    }

    pub async fn claim_and_fire_task(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.claim_and_fire_task(&id, &a))
            .await
    }

    pub async fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        let (i, s) = (id.to_owned(), status.to_owned());
        self.with_db(move |db| db.update_task_status(&i, &s)).await
    }

    pub async fn update_task_completed(&self, id: &str, result: Option<&str>) -> Result<bool> {
        let (i, r) = (id.to_owned(), result.map(|s| s.to_owned()));
        let a = self.agent_id.clone();
        self.with_db(move |db| db.update_task_completed(&i, &a, r.as_deref()))
            .await
    }

    pub async fn update_task_failed(&self, id: &str, error: &str) -> Result<()> {
        let id = id.to_string();
        let error = error.to_string();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.update_task_failed(&id, &a, &error))
            .await
    }

    pub async fn update_task_next_fire_at(&self, id: &str, next_fire_at: i64) -> Result<()> {
        let i = id.to_owned();
        self.with_db(move |db| db.update_task_next_fire_at(&i, next_fire_at))
            .await
    }

    pub async fn update_task_rescheduled(&self, id: &str, next_fire_at: i64) -> Result<()> {
        let i = id.to_owned();
        self.with_db(move |db| db.update_task_rescheduled(&i, next_fire_at))
            .await
    }

    pub async fn cancel_task(&self, id: &str) -> Result<bool> {
        let i = id.to_owned();
        let a = self.agent_id.clone();
        self.with_db(move |db| db.cancel_task(&i, &a)).await
    }

    pub async fn mark_tasks_expired(&self, now_unix: i64) -> Result<usize> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.mark_tasks_expired(now_unix, &id))
            .await
    }

    pub async fn count_pending_tasks(&self) -> Result<i64> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.count_pending_tasks(&id)).await
    }

    pub async fn get_pending_reminder_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_pending_reminder_tasks(&id))
            .await
    }

    pub async fn get_inject_context_tasks(&self) -> Result<Vec<Task>> {
        let id = self.agent_id.clone();
        self.with_db(move |db| db.get_inject_context_tasks(&id))
            .await
    }

    pub async fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        let i = id.to_owned();
        self.with_db(move |db| db.set_task_process_id(&i, process_id))
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

    // -- Conversation --

    pub async fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        let (a, r, c, ct) = (
            self.agent_id.clone(),
            role.to_owned(),
            content.to_owned(),
            channel_type.to_owned(),
        );
        self.with_db(move |db| db.save_message(&a, &r, &c, &ct))
            .await
    }

    pub async fn save_message_with_metadata(
        &self,
        role: &str,
        content: &str,
        channel_type: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        let (a, r, c, ct, m) = (
            self.agent_id.clone(),
            role.to_owned(),
            content.to_owned(),
            channel_type.to_owned(),
            metadata.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.save_message_with_metadata(&a, &r, &c, &ct, m.as_deref()))
            .await
    }

    pub async fn load_recent_messages(
        &self,
        limit: usize,
        channel_filter: Option<Vec<String>>,
    ) -> Result<Vec<ConversationMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| {
            let refs: Option<Vec<&str>> = channel_filter
                .as_ref()
                .map(|v| v.iter().map(|s| s.as_str()).collect());
            db.load_recent_messages(&a, limit, refs.as_deref())
        })
        .await
    }

    pub async fn load_conversation_summary(&self) -> Result<Option<ConversationMessage>> {
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
    ) -> Result<Vec<ConversationMessage>> {
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

    pub async fn seed_core_memory(&self, user_md_content: Option<String>) -> Result<()> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.seed_core_memory(&a, user_md_content.as_deref()))
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

    pub async fn last_user_message_time(&self) -> Result<Option<i64>> {
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

    pub async fn compact_old_memory_events(&self, days: u32) -> Result<usize> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.compact_old_memory_events(&a, days))
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

    // -- Cross-Channel Queries --

    pub async fn load_messages_after(
        &self,
        after_id: i64,
        channel_types: Option<Vec<String>>,
    ) -> Result<Vec<ConversationMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| {
            let refs: Option<Vec<&str>> = channel_types
                .as_ref()
                .map(|v| v.iter().map(|s| s.as_str()).collect());
            db.load_messages_after(&a, after_id, refs.as_deref())
        })
        .await
    }

    pub async fn max_message_id(&self) -> Result<i64> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.max_message_id(&a)).await
    }

    // -- Audit --

    pub async fn log_memory_event(
        &self,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: &str,
        reasoning: Option<&str>,
    ) -> Result<()> {
        let (a, sid, tn, tk, bv, av, r) = (
            self.agent_id.clone(),
            session_id.to_owned(),
            tool_name.to_owned(),
            target_key.to_owned(),
            before_value.map(|s| s.to_owned()),
            after_value.to_owned(),
            reasoning.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.log_memory_event(&a, &sid, &tn, &tk, bv.as_deref(), &av, r.as_deref())
        })
        .await
    }

    pub async fn get_memory_events(&self, session_id: &str) -> Result<Vec<MemoryEvent>> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.get_memory_events(&a, &s)).await
    }

    // -- Reflection --

    pub async fn get_conversations_since(
        &self,
        since_unix: i64,
    ) -> Result<Vec<ConversationMessage>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_conversations_since(&a, since_unix))
            .await
    }

    pub async fn get_memory_events_since(&self, since_unix: i64) -> Result<Vec<MemoryEvent>> {
        let a = self.agent_id.clone();
        self.with_db(move |db| db.get_memory_events_since(&a, since_unix))
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

    pub async fn count_memory_events_for_session(&self, session_id: &str) -> Result<i64> {
        let (a, s) = (self.agent_id.clone(), session_id.to_owned());
        self.with_db(move |db| db.count_memory_events_for_session(&a, &s))
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

    // -- Team Runs --

    pub async fn insert_team_run(
        &self,
        run_id: &str,
        team_name: &str,
        goal: &str,
        max_iterations: u32,
        started_at: i64,
    ) -> Result<()> {
        let (ri, tn, g) = (run_id.to_owned(), team_name.to_owned(), goal.to_owned());
        self.with_db(move |db| db.insert_team_run(&ri, &tn, &g, max_iterations, started_at))
            .await
    }

    pub async fn update_team_run(
        &self,
        run_id: &str,
        status: &str,
        failure_reason: Option<&str>,
        iteration: u32,
        deliverable: Option<&str>,
        ended_at: Option<i64>,
    ) -> Result<()> {
        let (ri, s, fr, d) = (
            run_id.to_owned(),
            status.to_owned(),
            failure_reason.map(|s| s.to_owned()),
            deliverable.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.update_team_run(&ri, &s, fr.as_deref(), iteration, d.as_deref(), ended_at)
        })
        .await
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

    // -- Team Messages --

    pub async fn insert_team_message(
        &self,
        run_id: &str,
        parent_id: Option<i64>,
        agent_name: Option<&str>,
        message_type: &str,
        content: &str,
        iteration: u32,
    ) -> Result<i64> {
        let (ri, an, mt, c) = (
            run_id.to_owned(),
            agent_name.map(|s| s.to_owned()),
            message_type.to_owned(),
            content.to_owned(),
        );
        self.with_db(move |db| {
            db.insert_team_message(&ri, parent_id, an.as_deref(), &mt, &c, iteration)
        })
        .await
    }

    pub async fn load_assignment_msg_ids(
        &self,
        run_id: &str,
        iteration: u32,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.load_assignment_msg_ids(&ri, iteration))
            .await
    }

    pub async fn load_team_messages(&self, run_id: &str) -> Result<Vec<TeamMessageRow>> {
        let ri = run_id.to_owned();
        self.with_db(move |db| db.load_team_messages(&ri)).await
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

    #[tokio::test]
    async fn test_async_save_and_load() {
        let db = test_async_db();
        db.save_message("user", "Hello!", "cli").await.unwrap();
        db.save_message("assistant", "Hi there!", "cli")
            .await
            .unwrap();
        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
    }

    #[tokio::test]
    async fn test_async_concurrent_reads() {
        let db = test_async_db();
        db.save_message("user", "Message 1", "cli").await.unwrap();
        db.save_message("user", "Message 2", "cli").await.unwrap();
        let mut handles = Vec::new();
        for _ in 0..5 {
            let db_clone = db.clone();
            handles.push(tokio::spawn(async move {
                db_clone.load_recent_messages(10, None).await.unwrap()
            }));
        }
        for handle in handles {
            let messages = handle.await.unwrap();
            assert_eq!(messages.len(), 2);
        }
    }

    #[tokio::test]
    async fn test_async_clone_shares_connection() {
        let db = test_async_db();
        let db2 = db.clone();
        db.save_message("user", "From clone 1", "cli")
            .await
            .unwrap();
        let messages = db2.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_async_db_survives_panic() {
        let db = test_async_db();
        db.save_message("user", "before panic", "cli")
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        db.inner
            .sender
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(Box::new(move |_db| {
                let _ = tx.send(());
                panic!("intentional test panic");
            }))
            .unwrap();
        rx.await.unwrap();
        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_async_core_memory_roundtrip() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let entries = db.get_all_core_memory().await.unwrap();
        assert!(!entries.is_empty());
        let entry = db.get_core_memory("user_summary").await.unwrap().unwrap();
        assert_eq!(entry.value, "New user. No information yet.");
    }

    #[tokio::test]
    async fn test_shutdown_joins_thread() {
        let db = test_async_db();
        db.save_message("user", "before shutdown", "cli")
            .await
            .unwrap();
        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        db.shutdown();
    }

    #[tokio::test]
    async fn test_shutdown_rejects_subsequent_operations() {
        let db = test_async_db();
        db.save_message("user", "msg", "cli").await.unwrap();
        db.shutdown();
        let result = db.load_recent_messages(10, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shut down"));
    }

    #[tokio::test]
    async fn test_shutdown_idempotent() {
        let db = test_async_db();
        db.save_message("user", "msg", "cli").await.unwrap();
        db.shutdown();
        db.shutdown();
    }

    #[tokio::test]
    async fn test_last_user_message_time_returns_i64() {
        let db = test_async_db();
        assert!(db.last_user_message_time().await.unwrap().is_none());
        db.save_message("user", "hello", "cli").await.unwrap();
        let ts = db.last_user_message_time().await.unwrap();
        assert!(ts.is_some());
        assert!(ts.unwrap() > 0);
    }

    #[tokio::test]
    async fn test_with_agent_scoping() {
        let db = test_async_db();
        // Register a second agent
        db.register_agent("agent2", "Agent 2", "").await.unwrap();
        let db2 = db.with_agent("agent2");
        db.save_message("user", "from main", "cli").await.unwrap();
        db2.save_message("user", "from agent2", "cli")
            .await
            .unwrap();
        let main_msgs = db.load_recent_messages(10, None).await.unwrap();
        let agent2_msgs = db2.load_recent_messages(10, None).await.unwrap();
        assert_eq!(main_msgs.len(), 1);
        assert_eq!(agent2_msgs.len(), 1);
        assert_eq!(main_msgs[0].content, "from main");
        assert_eq!(agent2_msgs[0].content, "from agent2");
    }

    #[tokio::test]
    async fn test_create_and_get_task() {
        let db = test_async_db();
        let task = NewTask {
            agent_id: "main".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Async reminder".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(9_999_999_999),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
        };
        let id = db.create_task(task).await.unwrap();
        let t = db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(t.label, "Async reminder");
        assert_eq!(t.status, "pending");
    }
}
