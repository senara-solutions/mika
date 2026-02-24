use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct CreateReminderTool;

#[async_trait(?Send)]
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

        let id = ctx.db.add_reminder(fire_at, message)?;

        ctx.db.log_memory_event(
            ctx.session_id,
            "create_reminder",
            &format!("reminder:{id}"),
            None,
            &format!("{fire_at} — {message}"),
            None,
        )?;

        Ok(ToolOutput::success(format!(
            "Reminder #{id} scheduled for {fire_at}."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_ctx, test_db};
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn test_create_reminder_valid() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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

        let reminders = db.get_pending_reminders().unwrap();
        assert_eq!(reminders.len(), 1);
        assert_eq!(reminders[0].message, "Year-end review");
    }

    #[tokio::test]
    async fn test_create_reminder_past_time() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
