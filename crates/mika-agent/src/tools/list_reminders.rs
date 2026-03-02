use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListRemindersTool;

#[async_trait]
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
        let reminders = ctx.db.get_pending_reminders().await?;

        if reminders.is_empty() {
            return Ok(ToolOutput::success("No active reminders."));
        }

        let mut output = String::from("Active reminders:\n");
        for r in &reminders {
            output.push_str(&format!(
                "- #{}: \"{}\" at {} (created: {})\n",
                r.id, r.message, r.display_fire_at(), r.created_at
            ));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_list_reminders_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No active reminders"));
    }

    #[tokio::test]
    async fn test_list_reminders_shows_active() {
        let harness = TestHarness::new();
        // 2099-01-01T00:00:00Z
        harness
            .db
            .add_reminder(4_070_908_800, "Meeting prep")
            .await
            .unwrap();
        // 2099-06-15T12:00:00Z
        harness
            .db
            .add_reminder(4_085_380_800, "Birthday party")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Meeting prep"));
        assert!(result.content.contains("Birthday party"));
    }

    #[tokio::test]
    async fn test_list_reminders_excludes_cancelled() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_reminder(4_070_908_800, "Cancelled one")
            .await
            .unwrap();
        harness.db.cancel_reminder(id).await.unwrap();
        harness
            .db
            .add_reminder(4_085_380_800, "Active one")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.content.contains("Cancelled one"));
        assert!(result.content.contains("Active one"));
    }
}
