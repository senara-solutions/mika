use anyhow::Result;
use chrono::Timelike;
use mika_common::claude::ClaudeClient;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tracing::{debug, info, warn};

use mika_common::embedding::EmbeddingClient;

use crate::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;

/// Manages reminder recovery and background polling.
///
/// - `recover()`: Fires past-due reminders on startup (max 5 to avoid blocking).
/// - `spawn_poller()`: Background task that polls every 60s for due reminders.
///
/// Owns all dependencies so it can be stored in `Arc<ReminderScheduler>` for AppState.
pub struct ReminderScheduler {
    pub db: AsyncDatabase,
    pub claude: ClaudeClient,
    pub tools: Arc<ToolRegistry>,
    pub skills: Arc<SkillRegistry>,
    pub home_dir: PathBuf,
    pub message_sender: Option<Arc<dyn MessageSender>>,
    pub embedding_client: Option<EmbeddingClient>,
    pub brave_api_key: Option<String>,
    pub skills_dirty: Arc<AtomicBool>,
    /// Per-agent lock to prevent concurrent agent loops in server mode.
    /// `None` in CLI mode (serialization handled by channel).
    /// When `Some`, `check_and_fire_reminders` uses `try_lock` and defers if busy.
    pub agent_lock: Option<Arc<tokio::sync::Mutex<()>>>,
    /// Cached reflection config loaded at construction time.
    /// Avoids re-reading identity.toml on every 60s poll tick.
    pub reflection_config: Option<crate::prompt::ReflectionConfig>,
}

impl ReminderScheduler {
    /// Recover pending reminders on startup.
    /// - Past-due reminders: fire immediately (max 5 to avoid blocking startup)
    /// - Future reminders: log count (poller handles them when due)
    ///
    /// Also prunes old heartbeat_sends records.
    pub async fn recover(&self) -> Result<()> {
        // Prune heartbeat sends older than 7 days to prevent unbounded growth
        if let Err(e) = self.db.prune_old_heartbeat_sends(7).await {
            warn!(error = %e, "failed to prune old heartbeat sends");
        }

        // Prune reflection runs older than 90 days
        if let Err(e) = self.db.prune_old_reflection_runs(90).await {
            warn!(error = %e, "failed to prune old reflection runs");
        }

        // Compact memory events older than 90 days into monthly summaries
        match self.db.compact_old_memory_events(90).await {
            Ok(deleted) if deleted > 0 => {
                info!(deleted, "compacted old memory events");
                if let Err(e) = self.db.vacuum().await {
                    warn!(error = %e, "failed to vacuum database");
                }
            }
            Ok(_) => {} // nothing to compact, skip VACUUM
            Err(e) => warn!(error = %e, "failed to compact old memory events"),
        }
        match self.db.db_size_bytes().await {
            Ok(size) if size > 500_000_000 => {
                warn!(size_bytes = size, "database size exceeds 500MB");
            }
            Err(e) => warn!(error = %e, "failed to check database size"),
            _ => {}
        }

        // Backfill search index if empty (one-time migration for existing data)
        match self.db.count_search_content().await {
            Ok(0) => {
                match self.db.get_all_facts_for_indexing().await {
                    Ok(facts) if !facts.is_empty() => {
                        info!(count = facts.len(), "backfilling search index");

                        // Index into FTS5 and track content_ids for embedding step
                        let mut content_ids: Vec<(i64, usize)> = Vec::new();
                        for (i, (source_type, source_id, content)) in facts.iter().enumerate() {
                            match self
                                .db
                                .index_content(source_type, Some(*source_id), content)
                                .await
                            {
                                Ok(content_id) => content_ids.push((content_id, i)),
                                Err(e) => {
                                    warn!(error = %e, source_type, "failed to index content during backfill");
                                }
                            }
                        }

                        // Batch-generate embeddings if client is available
                        if let Some(ref client) = self.embedding_client {
                            const BATCH_SIZE: usize = 100;
                            let mut indexed = 0;

                            for chunk in content_ids.chunks(BATCH_SIZE) {
                                let texts: Vec<&str> = chunk
                                    .iter()
                                    .map(|&(_, fact_idx)| facts[fact_idx].2.as_str())
                                    .collect();

                                match client.embed_batch(&texts).await {
                                    Ok(embeddings) => {
                                        for (j, embedding) in embeddings.into_iter().enumerate() {
                                            let (content_id, _) = chunk[j];
                                            if let Err(e) =
                                                self.db.index_embedding(content_id, embedding).await
                                            {
                                                warn!(error = %e, content_id, "failed to index embedding during backfill");
                                            } else {
                                                indexed += 1;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "failed to generate embeddings batch during backfill");
                                    }
                                }
                            }
                            info!(indexed, total = facts.len(), "backfill embeddings complete");
                        }
                        info!("search index backfill complete");
                    }
                    Ok(_) => {} // no facts to index
                    Err(e) => warn!(error = %e, "failed to get facts for backfill"),
                }
            }
            Err(e) => warn!(error = %e, "failed to check search content count"),
            _ => {} // index already populated
        }

        // Check for missed reflection (if enabled, past time, no record, within 4h window)
        {
            let identity = crate::prompt::load_identity_async(&self.home_dir).await;
            if let Some(config) = identity
                .reflection
                .as_ref()
                .filter(|c| c.enabled)
                .and_then(|c| c.parse_time().map(|t| (c, t)))
            {
                let (_, configured_time) = config;
                let tz_str = self
                    .db
                    .get_customer_config("timezone")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "UTC".to_string());
                let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
                let now_local = chrono::Utc::now().with_timezone(&tz);

                let current_minutes = now_local.hour() as i32 * 60 + now_local.minute() as i32;
                let configured_minutes =
                    configured_time.hour() as i32 * 60 + configured_time.minute() as i32;
                let elapsed_minutes = current_minutes - configured_minutes;

                // Past configured time and within 4-hour window
                if elapsed_minutes > 0
                    && elapsed_minutes <= 240
                    && matches!(self.db.last_reflection_run_today(&tz_str).await, Ok(false))
                {
                    info!("missed reflection detected on startup, firing now");
                    self.check_and_fire_reflection().await;
                }
            }
        }

        let past_due = self.db.get_past_due_reminders().await?;
        let future = self.db.get_future_reminders().await?;

        if past_due.is_empty() && future.is_empty() {
            info!("no pending reminders to recover");
            return Ok(());
        }

        info!(
            past_due = past_due.len(),
            future = future.len(),
            "recovering reminders"
        );

        // Fire past-due reminders (cap at 5 to avoid blocking startup)
        const MAX_RECOVERY_REMINDERS: usize = 5;
        for (i, reminder) in past_due.iter().enumerate() {
            if i >= MAX_RECOVERY_REMINDERS {
                warn!(
                    skipped = past_due.len() - MAX_RECOVERY_REMINDERS,
                    "too many past-due reminders, marking excess as failed"
                );
                for skipped in &past_due[MAX_RECOVERY_REMINDERS..] {
                    let _ = self.db.mark_reminder_failed(skipped.id).await;
                }
                break;
            }

            info!(
                id = reminder.id,
                fire_at = %reminder.fire_at,
                "firing past-due reminder"
            );
            let session_id = format!("reminder-recovery-{}", reminder.id);
            let params = SilentAgentParams {
                db: &self.db,
                claude: &self.claude,
                tools: &self.tools,
                skills: &self.skills,
                trigger: SilentTrigger::Reminder {
                    id: reminder.id,
                    message: reminder.message.clone(),
                },
                home_dir: &self.home_dir,
                session_id: &session_id,
                message_sender: self.message_sender.clone(),
                embedding_client: self.embedding_client.as_ref(),
                brave_api_key: self.brave_api_key.as_deref(),
                skills_dirty: &self.skills_dirty,
            };

            if let Err(e) = run_silent_agent(&params).await {
                warn!(
                    id = reminder.id,
                    error = %e,
                    "failed to fire past-due reminder"
                );
            }
        }

        if !future.is_empty() {
            info!(
                count = future.len(),
                "future reminders exist (poller will fire them when due)"
            );
        }

        Ok(())
    }

    /// Check for past-due reminders and fire them via silent agent.
    ///
    /// Called by the background poller every 60 seconds. Returns early
    /// with no log noise when there are no due reminders.
    ///
    /// In server mode (`agent_lock` is `Some`), acquires the per-agent lock
    /// via `try_lock` before firing any reminders. If the agent is busy
    /// (lock held by message handler or heartbeat), all reminders are deferred
    /// to the next poll cycle. This mirrors the heartbeat handler's pattern.
    async fn check_and_fire_reminders(&self) {
        let past_due = match self.db.get_past_due_reminders().await {
            Ok(reminders) => reminders,
            Err(e) => {
                warn!(error = %e, "failed to query past-due reminders");
                return;
            }
        };

        if past_due.is_empty() {
            return;
        }

        // In server mode, try to acquire the agent lock for the entire batch.
        // If busy, defer all reminders to the next cycle (60s).
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!(
                        count = past_due.len(),
                        "agent busy, deferring reminder firing to next cycle"
                    );
                    return;
                }
            }
        } else {
            None
        };

        // Cap per-cycle to avoid blocking the poller for too long
        const MAX_POLL_REMINDERS: usize = 10;
        let firing = if past_due.len() > MAX_POLL_REMINDERS {
            warn!(
                total = past_due.len(),
                firing = MAX_POLL_REMINDERS,
                "too many due reminders, deferring excess to next cycle"
            );
            &past_due[..MAX_POLL_REMINDERS]
        } else {
            &past_due
        };

        info!(count = firing.len(), "poller firing due reminders");

        for reminder in firing {
            let session_id = format!("reminder-{}", reminder.id);
            let params = SilentAgentParams {
                db: &self.db,
                claude: &self.claude,
                tools: &self.tools,
                skills: &self.skills,
                trigger: SilentTrigger::Reminder {
                    id: reminder.id,
                    message: reminder.message.clone(),
                },
                home_dir: &self.home_dir,
                session_id: &session_id,
                message_sender: self.message_sender.clone(),
                embedding_client: self.embedding_client.as_ref(),
                brave_api_key: self.brave_api_key.as_deref(),
                skills_dirty: &self.skills_dirty,
            };

            if let Err(e) = run_silent_agent(&params).await {
                warn!(
                    id = reminder.id,
                    error = %e,
                    "failed to fire reminder from poller"
                );
            }
        }
    }

    /// Check if it's time to run the daily reflection and fire it.
    ///
    /// Pre-filter checks (in order):
    /// 1. Reflection enabled in identity.toml
    /// 2. Past configured local time
    /// 3. Not already run today
    /// 4. User not active within 30 minutes
    /// 5. Conversations exist today
    /// 6. Agent lock available (server mode)
    async fn check_and_fire_reflection(&self) {
        // 1. Check cached reflection config
        let config = match self.reflection_config {
            Some(ref c) if c.enabled => c,
            _ => return,
        };

        // 2. Parse configured time
        let time = match config.parse_time() {
            Some(t) => t,
            None => return, // invalid time, warning logged at startup
        };

        // 3. Get user timezone, compute current local time
        let tz_str = self
            .db
            .get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string());
        let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
        let now_local = chrono::Utc::now().with_timezone(&tz);

        // Check if it's past the configured time
        if now_local.hour() < time.hour()
            || (now_local.hour() == time.hour() && now_local.minute() < time.minute())
        {
            return;
        }

        // 4. Check if already ran today
        match self.db.last_reflection_run_today(&tz_str).await {
            Ok(true) => return,
            Err(e) => {
                warn!(error = %e, "failed to check last reflection run");
                return;
            }
            _ => {}
        }

        // 5. Check recent user activity (defer if within 30 min)
        if let Ok(Some(last_msg_time)) = self.db.last_user_message_time().await {
            let parsed = chrono::DateTime::parse_from_rfc3339(&last_msg_time).or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&last_msg_time, "%Y-%m-%d %H:%M:%S")
                    .map(|ndt| ndt.and_local_timezone(chrono::Utc).unwrap().fixed_offset())
            });
            if let Ok(last_msg) = parsed {
                let elapsed = chrono::Utc::now().signed_duration_since(last_msg);
                if elapsed < chrono::TimeDelta::minutes(30) {
                    debug!("user active within 30 min, deferring reflection");
                    return;
                }
            }
        }

        // 6. Check conversation volume (skip if no activity today)
        let midnight_str = crate::db::today_midnight_utc(&tz_str)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let conversations = match self.db.get_conversations_since(&midnight_str).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "failed to query conversations for reflection");
                return;
            }
        };

        if conversations.is_empty() {
            debug!("no conversations today, skipping reflection");
            return;
        }

        // 7. Acquire agent lock (server mode)
        let _guard = if let Some(ref lock) = self.agent_lock {
            match lock.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    debug!("agent busy, deferring reflection to next cycle");
                    return;
                }
            }
        } else {
            None
        };

        // 8. Fire reflection
        let today_str = now_local.format("%Y-%m-%d").to_string();
        let session_id = format!("reflection-{today_str}");
        info!(session_id = %session_id, "firing daily reflection");

        let params = SilentAgentParams {
            db: &self.db,
            claude: &self.claude,
            tools: &self.tools,
            skills: &self.skills,
            trigger: SilentTrigger::Reflection,
            home_dir: &self.home_dir,
            session_id: &session_id,
            message_sender: self.message_sender.clone(),
            embedding_client: self.embedding_client.as_ref(),
            brave_api_key: self.brave_api_key.as_deref(),
            skills_dirty: &self.skills_dirty,
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(error = %e, "reflection run failed");
            let _ = self
                .db
                .record_reflection_run("failed", 0, Some(&e.to_string()))
                .await;
        }
    }

    /// Spawn a background task that polls for due reminders every 60 seconds.
    ///
    /// Uses `MissedTickBehavior::Skip` to prevent burst-firing after a slow
    /// reminder. Returns the `JoinHandle` so callers can abort on shutdown.
    pub fn spawn_poller(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Skip the first immediate tick (recover() already handled startup)
            interval.tick().await;

            loop {
                interval.tick().await;
                self.check_and_fire_reminders().await;
                self.check_and_fire_reflection().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_async_db, test_db};

    #[test]
    fn test_recover_no_reminders() {
        let db = test_db();
        let past_due = db.get_past_due_reminders().unwrap();
        let future = db.get_future_reminders().unwrap();
        assert!(past_due.is_empty());
        assert!(future.is_empty());
    }

    #[test]
    fn test_past_due_reminders_identified() {
        let db = test_db();
        // 2020-01-01T00:00:00Z as unix timestamp
        db.add_reminder(1_577_836_800, "Past due reminder").unwrap();
        // 2099-12-31T23:59:59Z as unix timestamp
        db.add_reminder(4_102_444_799, "Future reminder").unwrap();

        let past_due = db.get_past_due_reminders().unwrap();
        assert_eq!(past_due.len(), 1);
        assert_eq!(past_due[0].message, "Past due reminder");

        let future = db.get_future_reminders().unwrap();
        assert_eq!(future.len(), 1);
        assert_eq!(future[0].message, "Future reminder");
    }

    #[test]
    fn test_cancelled_reminders_excluded() {
        let db = test_db();
        let id = db.add_reminder(1_577_836_800, "Cancelled one").unwrap();
        db.cancel_reminder(id).unwrap();

        let past_due = db.get_past_due_reminders().unwrap();
        assert!(past_due.is_empty());
    }

    #[tokio::test]
    async fn test_check_and_fire_reminders_no_due() {
        let async_db = test_async_db();
        let claude = ClaudeClient::new(
            Some("test-key".to_string()),
            "claude-sonnet-4-6".to_string(),
            4096,
        )
        .expect("test API key should be valid");
        let tools = Arc::new(crate::tools::default_tools());
        let skills = Arc::new(SkillRegistry::empty());

        let scheduler = ReminderScheduler {
            db: async_db,
            claude,
            tools,
            skills,
            home_dir: PathBuf::from("/tmp/mika-test"),
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: Arc::new(AtomicBool::new(false)),
            agent_lock: None,
            reflection_config: None,
        };

        // Should succeed silently when no reminders are due
        scheduler.check_and_fire_reminders().await;
    }
}
