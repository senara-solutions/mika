use anyhow::Result;
use std::sync::{Arc, Mutex};

use crate::db::{
    Commitment, ConversationMessage, CoreMemoryEntry, Database, Event, FailedSend, MemoryEvent,
    Person, Preference, Reminder,
};

/// Async wrapper around `Database` that delegates all operations to
/// `tokio::task::spawn_blocking`, keeping the Tokio runtime unblocked.
///
/// Uses `Arc<Mutex<Database>>` internally. The Mutex is only held inside
/// `spawn_blocking` closures (on the blocking thread pool), never on a
/// Tokio worker thread. Concurrent access serializes through the Mutex,
/// but in practice there is no contention: the HTTP server serializes
/// agent loops with its own `Mutex<()>`, and CLi access is single-threaded.
#[derive(Clone)]
pub struct AsyncDatabase {
    inner: Arc<Mutex<Database>>,
}

impl AsyncDatabase {
    /// Wrap an existing `Database` for async access.
    pub fn new(db: Database) -> Self {
        Self {
            inner: Arc::new(Mutex::new(db)),
        }
    }

    /// Acquire a reference to the underlying sync `Database`.
    /// Only for use in tests or migration/setup code that runs before
    /// the async server starts.
    pub fn blocking(&self) -> Result<std::sync::MutexGuard<'_, Database>> {
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("database mutex poisoned"))
    }

    // -- Conversations --

    pub async fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let role = role.to_string();
        let content = content.to_string();
        let channel_type = channel_type.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.save_message(&role, &content, &channel_type)
        })
        .await?
    }

    pub async fn load_recent_messages(
        &self,
        limit: usize,
        channel_types: Option<&[&str]>,
    ) -> Result<Vec<ConversationMessage>> {
        let db = Arc::clone(&self.inner);
        let channel_types: Option<Vec<String>> =
            channel_types.map(|ts| ts.iter().map(|s| s.to_string()).collect());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            match &channel_types {
                Some(types) => {
                    let refs: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
                    db.load_recent_messages(limit, Some(&refs))
                }
                None => db.load_recent_messages(limit, None),
            }
        })
        .await?
    }

    // -- Core Memory --

    pub async fn get_core_memory(&self, key: &str) -> Result<Option<CoreMemoryEntry>> {
        let db = Arc::clone(&self.inner);
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_core_memory(&key)
        })
        .await?
    }

    pub async fn get_all_core_memory(&self) -> Result<Vec<CoreMemoryEntry>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_all_core_memory()
        })
        .await?
    }

    pub async fn set_core_memory(&self, key: &str, value: &str) -> Result<i32> {
        let db = Arc::clone(&self.inner);
        let key = key.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.set_core_memory(&key, &value)
        })
        .await?
    }

    pub async fn total_core_memory_tokens(&self) -> Result<i32> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.total_core_memory_tokens()
        })
        .await?
    }

    pub async fn seed_core_memory(&self, user_md_content: Option<&str>) -> Result<()> {
        let db = Arc::clone(&self.inner);
        let user_md_content = user_md_content.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.seed_core_memory(user_md_content.as_deref())
        })
        .await?
    }

    // -- People --

    pub async fn upsert_person(
        &self,
        name: &str,
        relationship: Option<&str>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let name = name.to_string();
        let relationship = relationship.map(|s| s.to_string());
        let notes = notes.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.upsert_person(&name, relationship.as_deref(), notes.as_deref())
        })
        .await?
    }

    pub async fn get_person(&self, name: &str) -> Result<Option<Person>> {
        let db = Arc::clone(&self.inner);
        let name = name.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_person(&name)
        })
        .await?
    }

    pub async fn list_people(&self) -> Result<Vec<Person>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.list_people()
        })
        .await?
    }

    // -- Commitments --

    pub async fn add_commitment(
        &self,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let description = description.to_string();
        let due_date = due_date.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.add_commitment(&description, due_date.as_deref(), person_id)
        })
        .await?
    }

    pub async fn list_commitments(&self, status: &str) -> Result<Vec<Commitment>> {
        let db = Arc::clone(&self.inner);
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.list_commitments(&status)
        })
        .await?
    }

    pub async fn update_commitment_status(&self, id: i64, status: &str) -> Result<()> {
        let db = Arc::clone(&self.inner);
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.update_commitment_status(id, &status)
        })
        .await?
    }

    // -- Preferences --

    pub async fn set_preference(&self, category: &str, value: &str) -> Result<()> {
        let db = Arc::clone(&self.inner);
        let category = category.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.set_preference(&category, &value)
        })
        .await?
    }

    pub async fn get_preference(&self, category: &str) -> Result<Option<String>> {
        let db = Arc::clone(&self.inner);
        let category = category.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_preference(&category)
        })
        .await?
    }

    pub async fn list_preferences(&self) -> Result<Vec<Preference>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.list_preferences()
        })
        .await?
    }

    // -- Events --

    pub async fn add_event(
        &self,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let description = description.to_string();
        let event_date = event_date.map(|s| s.to_string());
        let context = context.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.add_event(&description, event_date.as_deref(), context.as_deref())
        })
        .await?
    }

    pub async fn list_events(&self) -> Result<Vec<Event>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.list_events()
        })
        .await?
    }

    // -- Memory Events --

    pub async fn log_memory_event(
        &self,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: &str,
        reasoning: Option<&str>,
    ) -> Result<()> {
        let db = Arc::clone(&self.inner);
        let session_id = session_id.to_string();
        let tool_name = tool_name.to_string();
        let target_key = target_key.to_string();
        let before_value = before_value.map(|s| s.to_string());
        let after_value = after_value.to_string();
        let reasoning = reasoning.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.log_memory_event(
                &session_id,
                &tool_name,
                &target_key,
                before_value.as_deref(),
                &after_value,
                reasoning.as_deref(),
            )
        })
        .await?
    }

    pub async fn get_memory_events(&self, session_id: &str) -> Result<Vec<MemoryEvent>> {
        let db = Arc::clone(&self.inner);
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_memory_events(&session_id)
        })
        .await?
    }

    // -- Reminders --

    pub async fn add_reminder(&self, fire_at: &str, message: &str) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let fire_at = fire_at.to_string();
        let message = message.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.add_reminder(&fire_at, &message)
        })
        .await?
    }

    pub async fn get_pending_reminders(&self) -> Result<Vec<Reminder>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_pending_reminders()
        })
        .await?
    }

    pub async fn get_future_reminders(&self) -> Result<Vec<Reminder>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_future_reminders()
        })
        .await?
    }

    pub async fn get_past_due_reminders(&self) -> Result<Vec<Reminder>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_past_due_reminders()
        })
        .await?
    }

    pub async fn mark_reminder_delivered(&self, id: i64) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.mark_reminder_delivered(id)
        })
        .await?
    }

    pub async fn mark_reminder_failed(&self, id: i64) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.mark_reminder_failed(id)
        })
        .await?
    }

    pub async fn cancel_reminder(&self, id: i64) -> Result<bool> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.cancel_reminder(id)
        })
        .await?
    }

    // -- Heartbeat Rate Limiting --

    pub async fn record_heartbeat_send(&self) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.record_heartbeat_send()
        })
        .await?
    }

    pub async fn count_heartbeat_sends_today(&self, timezone_offset: &str) -> Result<u32> {
        let db = Arc::clone(&self.inner);
        let tz = timezone_offset.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.count_heartbeat_sends_today(&tz)
        })
        .await?
    }

    pub async fn count_heartbeat_sends_last_hour(&self) -> Result<u32> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.count_heartbeat_sends_last_hour()
        })
        .await?
    }

    pub async fn prune_old_heartbeat_sends(&self, days: u32) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.prune_old_heartbeat_sends(days)
        })
        .await?
    }

    // -- Conversation Compaction --

    pub async fn save_conversation_summary(
        &self,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let summary = summary.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.save_conversation_summary(&summary, compacted_through_id)
        })
        .await?
    }

    pub async fn delete_compacted_messages(&self, through_id: i64) -> Result<u32> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.delete_compacted_messages(through_id)
        })
        .await?
    }

    pub async fn load_conversation_summary(&self) -> Result<Option<ConversationMessage>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.load_conversation_summary()
        })
        .await?
    }

    pub async fn count_messages(&self) -> Result<usize> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.count_messages()
        })
        .await?
    }

    pub async fn load_messages_before_window(
        &self,
        window_size: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.load_messages_before_window(window_size)
        })
        .await?
    }

    pub async fn replace_with_summary(
        &self,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let summary = summary.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.replace_with_summary(&summary, compacted_through_id)
        })
        .await?
    }

    // -- Customer Config --

    pub async fn get_customer_config(&self, key: &str) -> Result<Option<String>> {
        let db = Arc::clone(&self.inner);
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_customer_config(&key)
        })
        .await?
    }

    pub async fn set_customer_config(&self, key: &str, value: &str) -> Result<()> {
        let db = Arc::clone(&self.inner);
        let key = key.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.set_customer_config(&key, &value)
        })
        .await?
    }

    // -- Failed Sends --

    pub async fn record_failed_send(&self, text: &str, request_id: Option<&str>) -> Result<i64> {
        let db = Arc::clone(&self.inner);
        let text = text.to_string();
        let request_id = request_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.record_failed_send(&text, request_id.as_deref())
        })
        .await?
    }

    pub async fn get_failed_sends(&self) -> Result<Vec<FailedSend>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.get_failed_sends()
        })
        .await?
    }

    pub async fn delete_failed_send(&self, id: i64) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.delete_failed_send(id)
        })
        .await?
    }

    pub async fn increment_failed_send_retry(&self, id: i64) -> Result<()> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.increment_failed_send_retry(id)
        })
        .await?
    }

    pub async fn last_user_message_time(&self) -> Result<Option<String>> {
        let db = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            db.last_user_message_time()
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn test_async_db() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().unwrap())
    }

    #[tokio::test]
    async fn test_save_and_load_messages() {
        let db = test_async_db();

        let id1 = db.save_message("user", "Hello!", "cli").await.unwrap();
        let id2 = db
            .save_message("assistant", "Hi there!", "cli")
            .await
            .unwrap();
        assert!(id1 > 0);
        assert!(id2 > id1);

        let messages = db.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello!");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Hi there!");
    }

    #[tokio::test]
    async fn test_core_memory_roundtrip() {
        let db = test_async_db();

        let tokens = db.set_core_memory("persona", "Mika").await.unwrap();
        assert!(tokens > 0);

        let entry = db.get_core_memory("persona").await.unwrap().unwrap();
        assert_eq!(entry.value, "Mika");

        let all = db.get_all_core_memory().await.unwrap();
        assert_eq!(all.len(), 1);

        let total = db.total_core_memory_tokens().await.unwrap();
        assert!(total > 0);
    }

    #[tokio::test]
    async fn test_seed_core_memory() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();

        let all = db.get_all_core_memory().await.unwrap();
        assert_eq!(all.len(), 4);
    }

    #[tokio::test]
    async fn test_person_crud() {
        let db = test_async_db();

        let id = db
            .upsert_person("Alice", Some("CTO"), Some("Likes coffee"))
            .await
            .unwrap();
        assert!(id > 0);

        let person = db.get_person("Alice").await.unwrap().unwrap();
        assert_eq!(person.canonical_name, "Alice");
        assert_eq!(person.relationship, Some("CTO".to_string()));

        let people = db.list_people().await.unwrap();
        assert_eq!(people.len(), 1);
    }

    #[tokio::test]
    async fn test_commitment_crud() {
        let db = test_async_db();

        let id = db
            .add_commitment("Review budget", Some("2026-03-01"), None)
            .await
            .unwrap();
        assert!(id > 0);

        let pending = db.list_commitments("pending").await.unwrap();
        assert_eq!(pending.len(), 1);

        db.update_commitment_status(id, "completed").await.unwrap();
        let pending = db.list_commitments("pending").await.unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_preference_crud() {
        let db = test_async_db();

        db.set_preference("food", "No shellfish").await.unwrap();
        let pref = db.get_preference("food").await.unwrap().unwrap();
        assert_eq!(pref, "No shellfish");

        let all = db.list_preferences().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_event_crud() {
        let db = test_async_db();

        let id = db
            .add_event("Board meeting", Some("2026-03-15"), Some("quarterly"))
            .await
            .unwrap();
        assert!(id > 0);

        let events = db.list_events().await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_memory_event_crud() {
        let db = test_async_db();

        db.log_memory_event(
            "sess-1",
            "store_fact",
            "person:Alice",
            None,
            "Alice — CTO",
            None,
        )
        .await
        .unwrap();

        let events = db.get_memory_events("sess-1").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "store_fact");
    }

    #[tokio::test]
    async fn test_concurrent_reads() {
        let db = test_async_db();
        db.save_message("user", "Hello", "cli").await.unwrap();

        // Spawn multiple concurrent reads
        let mut handles = Vec::new();
        for _ in 0..10 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                db.load_recent_messages(10, None).await.unwrap()
            }));
        }

        for handle in handles {
            let messages = handle.await.unwrap();
            assert_eq!(messages.len(), 1);
        }
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let db = test_async_db();
        let db2 = db.clone();

        db.save_message("user", "from db1", "cli").await.unwrap();
        let messages = db2.load_recent_messages(10, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "from db1");
    }

    #[tokio::test]
    async fn test_blocking_access() {
        let db = test_async_db();
        db.save_message("user", "test", "cli").await.unwrap();

        // blocking() should see the same data
        let guard = db.blocking().unwrap();
        let messages = guard.load_recent_messages(10, None).unwrap();
        assert_eq!(messages.len(), 1);
    }
}
