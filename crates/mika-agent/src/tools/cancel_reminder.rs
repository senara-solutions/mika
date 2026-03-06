use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct CancelReminderTool;

#[async_trait]
impl Tool for CancelReminderTool {
    fn name(&self) -> &str {
        "cancel_reminder"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cancel_reminder".to_string(),
            description: "Cancel a pending reminder by its ID (from list_reminders).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the reminder to cancel (from list_reminders)"
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let id = input["id"].as_str().unwrap_or("").trim();
        if id.is_empty() {
            return Ok(ToolOutput::error("'id' is required."));
        }

        let cancelled = ctx.db.cancel_task(id).await?;
        if cancelled {
            ctx.db
                .log_memory_event(
                    ctx.session_id,
                    "cancel_reminder",
                    &format!("task:{id}"),
                    None,
                    "cancelled",
                    None,
                )
                .await?;
            Ok(ToolOutput::success(format!(
                "Reminder {id} has been cancelled."
            )))
        } else {
            Ok(ToolOutput::error(format!(
                "Reminder {id} not found or not in pending status."
            )))
        }
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
    async fn test_cancel_reminder_success() {
        let harness = TestHarness::new();
        let id = add_reminder(&harness, 4_070_908_800, "To cancel").await;

        let ctx = harness.ctx();
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cancelled"));

        let pending = harness.db.get_pending_reminder_tasks().await.unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_cancel_reminder_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelReminderTool;

        let result = tool
            .execute(
                serde_json::json!({"id": "00000000-0000-0000-0000-000000000000"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_cancel_reminder_missing_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelReminderTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }
}
