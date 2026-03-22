use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::cancel_task::CancelTaskTool;
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
            description: "Cancel a pending reminder by UUID. Works for any task type.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the reminder to cancel."
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        CancelTaskTool.execute(input, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_reminder(harness: &TestHarness, fire_at_unix: &str, message: &str) -> String {
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
                next_fire_at: Some(fire_at_unix.to_string()),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: serde_json::json!({"text": message}).to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_cancel_reminder_success() {
        let harness = TestHarness::new();
        let id = add_reminder(&harness, "2099-01-01T00:00:00Z", "To cancel").await;

        let ctx = harness.ctx();
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cancelled"));

        let pending = harness.db.get_user_visible_tasks().await.unwrap();
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
