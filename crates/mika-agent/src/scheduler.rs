use anyhow::Result;
use mika_common::claude::ClaudeClient;
use std::path::Path;
use tracing::{info, warn};

use crate::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use crate::db::Database;
use crate::messaging::MessageSender;
use crate::tools::ToolRegistry;

/// Manages reminder recovery on startup.
///
/// Phase 1 (CLI): Fires past-due reminders immediately during `recover()`.
/// Future reminders are not timer-scheduled (no persistent runtime in CLI).
///
/// Phase 2 (HTTP server): Will add Tokio timer scheduling for future reminders.
pub struct ReminderScheduler<'a> {
    pub db: &'a Database,
    pub claude: &'a ClaudeClient,
    pub tools: &'a ToolRegistry,
    pub home_dir: &'a Path,
    pub message_sender: Option<&'a dyn MessageSender>,
}

impl ReminderScheduler<'_> {
    /// Recover pending reminders on startup.
    /// - Past-due reminders: fire immediately
    /// - Future reminders: log count (timer scheduling is Phase 2)
    pub async fn recover(&self) -> Result<()> {
        let past_due = self.db.get_past_due_reminders()?;
        let future = self.db.get_future_reminders()?;

        if past_due.is_empty() && future.is_empty() {
            info!("no pending reminders to recover");
            return Ok(());
        }

        info!(
            past_due = past_due.len(),
            future = future.len(),
            "recovering reminders"
        );

        // Fire past-due reminders immediately
        for reminder in &past_due {
            info!(
                id = reminder.id,
                fire_at = %reminder.fire_at,
                "firing past-due reminder"
            );
            let session_id = format!("reminder-recovery-{}", reminder.id);
            let params = SilentAgentParams {
                db: self.db,
                claude: self.claude,
                tools: self.tools,
                trigger: SilentTrigger::Reminder {
                    id: reminder.id,
                    message: reminder.message.clone(),
                },
                home_dir: self.home_dir,
                session_id: &session_id,
                message_sender: self.message_sender,
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
