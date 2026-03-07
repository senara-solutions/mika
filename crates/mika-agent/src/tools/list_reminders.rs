use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_unix_ts;

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
        let tasks = ctx.db.get_user_visible_tasks().await?;

        if tasks.is_empty() {
            return Ok(ToolOutput::success("No active reminders."));
        }

        let mut output = String::from("Active reminders:\n");
        for t in &tasks {
            let fire_at = t
                .next_fire_at
                .map(format_unix_ts)
                .unwrap_or_else(|| "unknown".to_string());
            output.push_str(&format!("- {}: \"{}\" at {}\n", t.id, t.label, fire_at));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_reminder(harness: &TestHarness, fire_at_unix: i64, message: &str) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: message.to_string(),
                trigger_type: "time".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some(fire_at_unix),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: serde_json::json!({"text": message}).to_string(),
                input_context: None,
                created_by_session: None,
            })
            .await
            .unwrap()
    }

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
        add_reminder(&harness, 4_070_908_800, "Meeting prep").await;
        // 2099-06-15T12:00:00Z
        add_reminder(&harness, 4_085_380_800, "Birthday party").await;

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
        let id = add_reminder(&harness, 4_070_908_800, "Cancelled one").await;
        harness.db.cancel_task(&id).await.unwrap();
        add_reminder(&harness, 4_085_380_800, "Active one").await;

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.content.contains("Cancelled one"));
        assert!(result.content.contains("Active one"));
    }
}
