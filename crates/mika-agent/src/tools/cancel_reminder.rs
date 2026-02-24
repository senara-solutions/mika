use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct CancelReminderTool;

#[async_trait(?Send)]
impl Tool for CancelReminderTool {
    fn name(&self) -> &str {
        "cancel_reminder"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cancel_reminder".to_string(),
            description: "Cancel a pending reminder by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The ID of the reminder to cancel (from list_reminders)"
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let id = input["id"].as_i64().unwrap_or(0);
        if id <= 0 {
            return Ok(ToolOutput::error(
                "'id' is required and must be a positive integer.",
            ));
        }

        let cancelled = ctx.db.cancel_reminder(id).await?;
        if cancelled {
            ctx.db.log_memory_event(
                ctx.session_id,
                "cancel_reminder",
                &format!("reminder:{id}"),
                None,
                "cancelled",
                None,
            ).await?;
            Ok(ToolOutput::success(format!(
                "Reminder #{id} has been cancelled."
            )))
        } else {
            Ok(ToolOutput::error(format!(
                "Reminder #{id} not found or not in pending status."
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_async_db, test_ctx};
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn test_cancel_reminder_success() {
        let db = test_async_db();
        let id = db
            .add_reminder("2099-01-01T00:00:00Z", "To cancel")
            .await
            .unwrap();

        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cancelled"));

        let pending = db.get_pending_reminders().await.unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_cancel_reminder_not_found() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": 999}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_cancel_reminder_already_delivered() {
        let db = test_async_db();
        let id = db
            .add_reminder("2020-01-01T00:00:00Z", "Already delivered")
            .await
            .unwrap();
        db.mark_reminder_delivered(id).await.unwrap();

        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found or not in pending"));
    }

    #[tokio::test]
    async fn test_cancel_reminder_negative_id() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = CancelReminderTool;

        let result = tool
            .execute(serde_json::json!({"id": -5}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }

    #[tokio::test]
    async fn test_cancel_reminder_missing_id() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = CancelReminderTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }
}
