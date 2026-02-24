use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use tokio::sync::oneshot;

use crate::db::{
    Commitment, ConversationMessage, CoreMemoryEntry, Database, Event, FailedSend, MemoryEvent,
    Person, Preference, Reminder,
};

type DbClosure = Box<dyn FnOnce(&Database) + Send>;

/// Async wrapper around [`Database`] using a dedicated OS thread.
///
/// All SQL operations run on a single background thread that owns the
/// `Database` (and its `rusqlite::Connection`). Callers send closures over
/// an `mpsc` channel; results come back via `tokio::sync::oneshot`.
///
/// `Clone` is cheap (clones the inner `Arc`). All clones share the
/// same background thread and connection.
#[derive(Clone)]
pub struct AsyncDatabase {
    inner: Arc<AsyncDatabaseInner>,
}

struct AsyncDatabaseInner {
    /// Wrapped in `Option` so `shutdown()` can drop it to signal the thread.
    sender: Mutex<Option<mpsc::Sender<DbClosure>>>,
    /// Handle to the background DB thread, used for graceful shutdown.
    /// `None` after `shutdown()` has joined the thread.
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}

impl AsyncDatabase {
    /// Spawn a dedicated OS thread that owns `db` and processes closures.
    ///
    /// If a closure panics, the thread logs the error and continues
    /// processing subsequent closures (see [`std::panic::catch_unwind`]).
    pub fn new(db: Database) -> Self {
        let (tx, rx) = mpsc::channel::<DbClosure>();
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
        }
    }

    /// Open a database at `path` and wrap it in an async handle.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::open(path)?;
        Ok(Self::new(db))
    }

    /// Gracefully shut down the database thread.
    ///
    /// Drops the internal sender (so the thread's `recv()` loop exits)
    /// and joins the thread. Safe to call multiple times; subsequent
    /// calls are no-ops. Any in-flight or future `with_db` calls will
    /// receive an error once the sender is dropped.
    pub fn shutdown(&self) {
        // Drop the sender first so the background thread's rx.recv() returns Err.
        {
            let mut sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
            *sender_guard = None;
        }
        // Now join the thread (it will exit promptly since the channel is closed).
        let mut handle_guard = self
            .inner
            .thread_handle
            .lock()
            .expect("handle lock poisoned");
        if let Some(handle) = handle_guard.take() {
            let _ = handle.join();
        }
    }

    /// Send a closure to the background thread and await its result.
    async fn with_db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Database) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        {
            let sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
            let sender = sender_guard
                .as_ref()
                .ok_or_else(|| anyhow!("database has been shut down"))?;
            sender
                .send(Box::new(move |db| {
                    let _ = tx.send(f(db));
                }))
                .map_err(|_| anyhow!("database thread has stopped"))?;
        }
        rx.await
            .map_err(|_| anyhow!("database thread dropped reply"))?
    }

    // -- Conversation --

    pub async fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        let (r, c, ct) = (role.to_owned(), content.to_owned(), channel_type.to_owned());
        self.with_db(move |db| db.save_message(&r, &c, &ct)).await
    }

    pub async fn load_recent_messages(
        &self,
        limit: usize,
        channel_filter: Option<Vec<String>>,
    ) -> Result<Vec<ConversationMessage>> {
        self.with_db(move |db| {
            let refs: Option<Vec<&str>> = channel_filter
                .as_ref()
                .map(|v| v.iter().map(|s| s.as_str()).collect());
            db.load_recent_messages(limit, refs.as_deref())
        })
        .await
    }

    pub async fn load_conversation_summary(&self) -> Result<Option<ConversationMessage>> {
        self.with_db(|db| db.load_conversation_summary()).await
    }

    pub async fn count_messages(&self) -> Result<usize> {
        self.with_db(|db| db.count_messages()).await
    }

    pub async fn load_messages_before_window(
        &self,
        window_size: usize,
    ) -> Result<Vec<ConversationMessage>> {
        self.with_db(move |db| db.load_messages_before_window(window_size))
            .await
    }

    pub async fn replace_with_summary(
        &self,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let s = summary.to_owned();
        self.with_db(move |db| db.replace_with_summary(&s, compacted_through_id))
            .await
    }

    // -- Core Memory --

    pub async fn get_core_memory(&self, key: &str) -> Result<Option<CoreMemoryEntry>> {
        let k = key.to_owned();
        self.with_db(move |db| db.get_core_memory(&k)).await
    }

    pub async fn get_all_core_memory(&self) -> Result<Vec<CoreMemoryEntry>> {
        self.with_db(|db| db.get_all_core_memory()).await
    }

    pub async fn set_core_memory(&self, key: &str, value: &str) -> Result<i32> {
        let (k, v) = (key.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_core_memory(&k, &v)).await
    }

    pub async fn seed_core_memory(&self, user_md_content: Option<String>) -> Result<()> {
        self.with_db(move |db| db.seed_core_memory(user_md_content.as_deref()))
            .await
    }

    pub async fn total_core_memory_tokens(&self) -> Result<i32> {
        self.with_db(|db| db.total_core_memory_tokens()).await
    }

    // -- People --

    pub async fn upsert_person(
        &self,
        name: &str,
        relationship: Option<&str>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let (n, r, no) = (
            name.to_owned(),
            relationship.map(|s| s.to_owned()),
            notes.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.upsert_person(&n, r.as_deref(), no.as_deref()))
            .await
    }

    pub async fn get_person(&self, name: &str) -> Result<Option<Person>> {
        let n = name.to_owned();
        self.with_db(move |db| db.get_person(&n)).await
    }

    pub async fn list_people(&self) -> Result<Vec<Person>> {
        self.with_db(|db| db.list_people()).await
    }

    pub async fn search_people(&self, query: &str) -> Result<Vec<Person>> {
        let q = query.to_owned();
        self.with_db(move |db| db.search_people(&q)).await
    }

    // -- Commitments --

    pub async fn add_commitment(
        &self,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        let (d, dd) = (description.to_owned(), due_date.map(|s| s.to_owned()));
        self.with_db(move |db| db.add_commitment(&d, dd.as_deref(), person_id))
            .await
    }

    pub async fn list_commitments(&self, status: &str) -> Result<Vec<Commitment>> {
        let s = status.to_owned();
        self.with_db(move |db| db.list_commitments(&s)).await
    }

    pub async fn update_commitment_status(&self, id: i64, status: &str) -> Result<bool> {
        let s = status.to_owned();
        self.with_db(move |db| db.update_commitment_status(id, &s))
            .await
    }

    pub async fn get_commitment_status(&self, id: i64) -> Result<Option<String>> {
        self.with_db(move |db| db.get_commitment_status(id)).await
    }

    pub async fn search_commitments(&self, query: &str) -> Result<Vec<Commitment>> {
        let q = query.to_owned();
        self.with_db(move |db| db.search_commitments(&q)).await
    }

    // -- Preferences --

    pub async fn set_preference(&self, category: &str, value: &str) -> Result<()> {
        let (c, v) = (category.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_preference(&c, &v)).await
    }

    pub async fn get_preference(&self, category: &str) -> Result<Option<String>> {
        let c = category.to_owned();
        self.with_db(move |db| db.get_preference(&c)).await
    }

    pub async fn list_preferences(&self) -> Result<Vec<Preference>> {
        self.with_db(|db| db.list_preferences()).await
    }

    pub async fn search_preferences(&self, query: &str) -> Result<Vec<Preference>> {
        let q = query.to_owned();
        self.with_db(move |db| db.search_preferences(&q)).await
    }

    // -- Events --

    pub async fn add_event(
        &self,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        let (d, ed, c) = (
            description.to_owned(),
            event_date.map(|s| s.to_owned()),
            context.map(|s| s.to_owned()),
        );
        self.with_db(move |db| db.add_event(&d, ed.as_deref(), c.as_deref()))
            .await
    }

    pub async fn list_events(&self) -> Result<Vec<Event>> {
        self.with_db(|db| db.list_events()).await
    }

    pub async fn search_events(&self, query: &str) -> Result<Vec<Event>> {
        let q = query.to_owned();
        self.with_db(move |db| db.search_events(&q)).await
    }

    // -- Reminders --

    pub async fn add_reminder(&self, fire_at: &str, message: &str) -> Result<i64> {
        let (f, m) = (fire_at.to_owned(), message.to_owned());
        self.with_db(move |db| db.add_reminder(&f, &m)).await
    }

    pub async fn get_pending_reminders(&self) -> Result<Vec<Reminder>> {
        self.with_db(|db| db.get_pending_reminders()).await
    }

    pub async fn get_future_reminders(&self) -> Result<Vec<Reminder>> {
        self.with_db(|db| db.get_future_reminders()).await
    }

    pub async fn get_past_due_reminders(&self) -> Result<Vec<Reminder>> {
        self.with_db(|db| db.get_past_due_reminders()).await
    }

    pub async fn mark_reminder_delivered(&self, id: i64) -> Result<()> {
        self.with_db(move |db| db.mark_reminder_delivered(id)).await
    }

    pub async fn mark_reminder_failed(&self, id: i64) -> Result<()> {
        self.with_db(move |db| db.mark_reminder_failed(id)).await
    }

    pub async fn cancel_reminder(&self, id: i64) -> Result<bool> {
        self.with_db(move |db| db.cancel_reminder(id)).await
    }

    pub async fn search_reminders(&self, query: &str) -> Result<Vec<Reminder>> {
        let q = query.to_owned();
        self.with_db(move |db| db.search_reminders(&q)).await
    }

    // -- Housekeeping --

    pub async fn prune_old_heartbeat_sends(&self, days: u32) -> Result<()> {
        self.with_db(move |db| db.prune_old_heartbeat_sends(days))
            .await
    }

    pub async fn record_heartbeat_send(&self) -> Result<()> {
        self.with_db(|db| db.record_heartbeat_send()).await
    }

    pub async fn count_heartbeat_sends_last_hour(&self) -> Result<u32> {
        self.with_db(|db| db.count_heartbeat_sends_last_hour())
            .await
    }

    pub async fn count_heartbeat_sends_today(&self, timezone: &str) -> Result<u32> {
        let tz = timezone.to_owned();
        self.with_db(move |db| db.count_heartbeat_sends_today(&tz))
            .await
    }

    pub async fn last_user_message_time(&self) -> Result<Option<String>> {
        self.with_db(|db| db.last_user_message_time()).await
    }

    // -- Failed Sends --

    pub async fn save_failed_send(&self, text: &str, request_id: Option<&str>) -> Result<i64> {
        let (t, r) = (text.to_owned(), request_id.map(|s| s.to_owned()));
        self.with_db(move |db| db.save_failed_send(&t, r.as_deref()))
            .await
    }

    pub async fn get_pending_failed_sends(&self, limit: usize) -> Result<Vec<FailedSend>> {
        self.with_db(move |db| db.get_pending_failed_sends(limit))
            .await
    }

    pub async fn delete_failed_send(&self, id: i64) -> Result<()> {
        self.with_db(move |db| db.delete_failed_send(id)).await
    }

    pub async fn increment_failed_send_retry(&self, id: i64) -> Result<()> {
        self.with_db(move |db| db.increment_failed_send_retry(id))
            .await
    }

    pub async fn compact_old_memory_events(&self, days: u32) -> Result<usize> {
        self.with_db(move |db| db.compact_old_memory_events(days))
            .await
    }

    pub async fn db_size_bytes(&self) -> Result<u64> {
        self.with_db(|db| db.db_size_bytes()).await
    }

    pub async fn vacuum(&self) -> Result<()> {
        self.with_db(|db| db.vacuum()).await
    }

    // -- Customer Config --

    pub async fn get_customer_config(&self, key: &str) -> Result<Option<String>> {
        let k = key.to_owned();
        self.with_db(move |db| db.get_customer_config(&k)).await
    }

    pub async fn set_customer_config(&self, key: &str, value: &str) -> Result<()> {
        let (k, v) = (key.to_owned(), value.to_owned());
        self.with_db(move |db| db.set_customer_config(&k, &v)).await
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
        let (sid, tn, tk, bv, av, r) = (
            session_id.to_owned(),
            tool_name.to_owned(),
            target_key.to_owned(),
            before_value.map(|s| s.to_owned()),
            after_value.to_owned(),
            reasoning.map(|s| s.to_owned()),
        );
        self.with_db(move |db| {
            db.log_memory_event(&sid, &tn, &tk, bv.as_deref(), &av, r.as_deref())
        })
        .await
    }

    pub async fn get_memory_events(&self, session_id: &str) -> Result<Vec<MemoryEvent>> {
        let s = session_id.to_owned();
        self.with_db(move |db| db.get_memory_events(&s)).await
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
        assert_eq!(messages[0].content, "Hello!");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Hi there!");
    }

    #[tokio::test]
    async fn test_async_concurrent_reads() {
        let db = test_async_db();
        db.save_message("user", "Message 1", "cli").await.unwrap();
        db.save_message("user", "Message 2", "cli").await.unwrap();

        // Spawn multiple concurrent reads
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

        // Write via one clone, read via the other
        db.save_message("user", "From clone 1", "cli")
            .await
            .unwrap();
        let messages = db2.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "From clone 1");
    }

    #[tokio::test]
    async fn test_async_db_survives_panic() {
        let db = test_async_db();
        db.save_message("user", "before panic", "cli")
            .await
            .unwrap();

        // Send a closure that panics — the thread should catch it
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
        // Wait for the panicking closure to execute
        rx.await.unwrap();

        // Thread should still be alive — this call should succeed
        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "before panic");
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

        // Verify data is there before shutdown
        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);

        // Shutdown should complete without hanging (only clone, so thread joins)
        db.shutdown();
    }

    #[tokio::test]
    async fn test_shutdown_rejects_subsequent_operations() {
        let db = test_async_db();
        db.save_message("user", "msg", "cli").await.unwrap();

        db.shutdown();

        // After shutdown, operations should return an error
        let result = db.load_recent_messages(10, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("shut down"));
    }

    #[tokio::test]
    async fn test_shutdown_idempotent() {
        let db = test_async_db();
        db.save_message("user", "msg", "cli").await.unwrap();

        // Calling shutdown multiple times should not panic
        db.shutdown();
        db.shutdown();
    }
}
