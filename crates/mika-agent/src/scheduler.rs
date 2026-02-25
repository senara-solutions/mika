use anyhow::Result;
use mika_common::claude::ClaudeClient;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;

/// Manages reminder recovery on startup.
///
/// Phase 1 (CLI): Fires past-due reminders immediately during `recover()`.
/// Future reminders are not timer-scheduled (no persistent runtime in CLI).
///
/// Phase 2 (HTTP server): Will add Tokio timer scheduling for future reminders.
///
/// Owns all dependencies so it can be stored in `Arc<ReminderScheduler>` for AppState.
pub struct ReminderScheduler {
    pub db: AsyncDatabase,
    pub claude: ClaudeClient,
    pub tools: Arc<ToolRegistry>,
    pub skills: Arc<SkillRegistry>,
    pub home_dir: PathBuf,
    pub message_sender: Option<Arc<dyn MessageSender>>,
}

impl ReminderScheduler {
    /// Recover pending reminders on startup.
    /// - Past-due reminders: fire immediately (max 5 to avoid blocking startup)
    /// - Future reminders: log count (timer scheduling is Phase 2)
    ///
    /// Also prunes old heartbeat_sends records.
    pub async fn recover(&self) -> Result<()> {
        // Prune heartbeat sends older than 7 days to prevent unbounded growth
        if let Err(e) = self.db.prune_old_heartbeat_sends(7).await {
            warn!(error = %e, "failed to prune old heartbeat sends");
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
                "future reminders exist (timer scheduling is Phase 2)"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_utils::test_helpers::test_db;

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
        db.add_reminder("2020-01-01T00:00:00Z", "Past due reminder")
            .unwrap();
        db.add_reminder("2099-12-31T23:59:59Z", "Future reminder")
            .unwrap();

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
        let id = db
            .add_reminder("2020-01-01T00:00:00Z", "Cancelled one")
            .unwrap();
        db.cancel_reminder(id).unwrap();

        let past_due = db.get_past_due_reminders().unwrap();
        assert!(past_due.is_empty());
    }
}
