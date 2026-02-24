use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListRemindersTool;

#[async_trait(?Send)]
impl Tool for ListRemindersTool {
    fn name(&self) -> &str {
        "list_reminders"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_reminders".to_string(),
            description: "List all active (pending) reminders.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let reminders = ctx.db.list_active_reminders()?;

        if reminders.is_empty() {
            return Ok(ToolOutput::success("No active reminders."));
        }

        let mut output = String::from("Active reminders:\n");
        for r in &reminders {
            output.push_str(&format!(
                "- #{}: \"{}\" at {} (status: {})\n",
                r.id, r.message, r.fire_at, r.status
            ));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_ctx, test_db};
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn test_list_reminders_empty() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No active reminders"));
    }

    #[tokio::test]
    async fn test_list_reminders_shows_active() {
        let db = test_db();
        db.add_reminder("2099-01-01T00:00:00Z", "Meeting prep")
            .unwrap();
        db.add_reminder("2099-06-15T12:00:00Z", "Birthday party")
            .unwrap();

        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Meeting prep"));
        assert!(result.content.contains("Birthday party"));
    }

    #[tokio::test]
    async fn test_list_reminders_excludes_cancelled() {
        let db = test_db();
        let id = db
            .add_reminder("2099-01-01T00:00:00Z", "Cancelled one")
            .unwrap();
        db.cancel_reminder(id).unwrap();
        db.add_reminder("2099-06-15T12:00:00Z", "Active one")
            .unwrap();

        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.content.contains("Cancelled one"));
        assert!(result.content.contains("Active one"));
    }
}
