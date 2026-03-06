use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use crate::db::NewTask;
use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct CreateReminderTool;

#[async_trait]
impl Tool for CreateReminderTool {
    fn name(&self) -> &str {
        "create_reminder"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_reminder".to_string(),
            description: "Schedule a reminder for the user at a specific time. \
                The fire_at parameter must be an ISO 8601 datetime string \
                (e.g., '2026-02-25T15:00:00Z'). Parse the user's natural language \
                time into ISO 8601 before calling this tool."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fire_at": {
                        "type": "string",
                        "description": "ISO 8601 datetime (UTC) when the reminder should fire"
                    },
                    "message": {
                        "type": "string",
                        "description": "The reminder message to deliver to the user"
                    }
                },
                "required": ["fire_at", "message"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let fire_at = input["fire_at"].as_str().unwrap_or("");
        let message = input["message"].as_str().unwrap_or("");

        if fire_at.is_empty() {
            return Ok(ToolOutput::error("'fire_at' is required."));
        }
        if fire_at.len() > 64 {
            return Ok(ToolOutput::error("'fire_at' is too long."));
        }
        if message.is_empty() {
            return Ok(ToolOutput::error("'message' is required."));
        }
        if message.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'message' too long: {} characters (max: {MAX_INPUT_LEN})",
                message.len()
            )));
        }

        // Validate ISO 8601
        let parsed = match chrono::DateTime::parse_from_rfc3339(fire_at) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                return Ok(ToolOutput::error(
                    "Invalid ISO 8601 datetime. Use format like '2026-02-25T15:00:00Z'.",
                ));
            }
        };

        if parsed <= Utc::now() {
            return Ok(ToolOutput::error("Reminder time must be in the future."));
        }

        let timestamp = parsed.timestamp();
        let action_config = serde_json::json!({"text": message}).to_string();
        let task = NewTask {
            agent_id: ctx.db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: message.to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(timestamp),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config,
            input_context: None,
            created_by_session: Some(ctx.session_id.to_string()),
        };

        let id = ctx.db.create_task(task).await?;

        let display_time = parsed.format("%Y-%m-%d %H:%M:%S UTC");
        ctx.db
            .log_memory_event(
                ctx.session_id,
                "create_reminder",
                &format!("task:{id}"),
                None,
                &format!("{display_time} — {message}"),
                None,
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Reminder {id} scheduled for {display_time}."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_create_reminder_valid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "message": "Year-end review"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("scheduled"));

        let reminders = harness.db.get_pending_reminder_tasks().await.unwrap();
        assert_eq!(reminders.len(), 1);
        assert_eq!(reminders[0].label, "Year-end review");
    }

    #[tokio::test]
    async fn test_create_reminder_past_time() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2020-01-01T00:00:00Z",
                    "message": "Too late"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("future"));
    }

    #[tokio::test]
    async fn test_create_reminder_invalid_datetime() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "not-a-date",
                    "message": "Nope"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid ISO 8601"));
    }

    #[tokio::test]
    async fn test_create_reminder_missing_fields() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(serde_json::json!({"message": "no fire_at"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);

        let result = tool
            .execute(serde_json::json!({"fire_at": "2099-12-31T23:59:59Z"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
    }
}
