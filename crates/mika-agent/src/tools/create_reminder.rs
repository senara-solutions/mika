use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::NewTask;
use crate::task_engine::cron::next_fire_from_cron;

pub struct CreateReminderTool;

#[async_trait]
impl Tool for CreateReminderTool {
    fn name(&self) -> &str {
        "create_reminder"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_reminder".to_string(),
            description: "Schedule a reminder for the user. For one-shot reminders, \
                provide fire_at (ISO 8601 datetime in UTC). For periodic reminders, \
                provide cron_expr (6-field cron expression with seconds field first, \
                e.g. '0 0 9 * * 1' for every Monday at 9am UTC). \
                Parse the user's natural language into the appropriate format \
                before calling this tool."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fire_at": {
                        "type": "string",
                        "description": "ISO 8601 datetime (UTC) when the reminder should fire. Required for one-shot reminders, ignored for periodic."
                    },
                    "message": {
                        "type": "string",
                        "description": "The reminder message to deliver to the user"
                    },
                    "cron_expr": {
                        "type": "string",
                        "description": "6-field cron expression (seconds first) for periodic reminders. Example: '0 0 9 * * 1' = every Monday at 9am UTC, '0 0 18 * * *' = daily at 6pm UTC, '0 0 9 * * 1-5' = weekdays at 9am UTC."
                    }
                },
                "required": ["message"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let fire_at = input["fire_at"].as_str().unwrap_or("");
        let message = input["message"].as_str().unwrap_or("");
        let cron_expr_input = input["cron_expr"].as_str().unwrap_or("");

        if message.is_empty() {
            return Ok(ToolOutput::error("'message' is required."));
        }
        if message.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'message' too long: {} characters (max: {MAX_INPUT_LEN})",
                message.len()
            )));
        }

        // Determine scheduling mode: periodic (cron) or one-shot (fire_at)
        let (trigger_type, cron_expr, next_fire_at, display) = if !cron_expr_input.is_empty() {
            // Periodic reminder
            if cron_expr_input.len() > 128 {
                return Ok(ToolOutput::error("'cron_expr' is too long."));
            }

            let now = Utc::now().timestamp();
            let next_fire = match next_fire_from_cron(cron_expr_input, now) {
                Ok(ts) => ts,
                Err(_) => {
                    return Ok(ToolOutput::error(
                        "Invalid cron expression. Use 6-field format (seconds first), \
                         e.g. '0 0 9 * * 1' for every Monday at 9am UTC.",
                    ));
                }
            };

            // Reject cron expressions that fire more frequently than once per minute
            const MIN_INTERVAL_SECS: i64 = 60;
            if next_fire - now < MIN_INTERVAL_SECS {
                // Check second interval to confirm (the first might be short due to alignment)
                if let Ok(second_fire) = next_fire_from_cron(cron_expr_input, next_fire)
                    && second_fire - next_fire < MIN_INTERVAL_SECS
                {
                    return Ok(ToolOutput::error(
                        "Cron expression fires too frequently. Minimum interval is 1 minute.",
                    ));
                }
            }

            (
                "recurring",
                Some(cron_expr_input.to_string()),
                next_fire,
                format!("periodic ({cron_expr_input})"),
            )
        } else {
            // One-shot reminder
            if fire_at.is_empty() {
                return Ok(ToolOutput::error(
                    "'fire_at' is required for one-shot reminders (or provide 'cron_expr' for periodic).",
                ));
            }
            if fire_at.len() > 64 {
                return Ok(ToolOutput::error("'fire_at' is too long."));
            }

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
            let display_time = parsed.format("%Y-%m-%d %H:%M:%S UTC").to_string();
            ("time", None, timestamp, display_time)
        };

        let action_config = serde_json::json!({"text": message}).to_string();
        let task = NewTask {
            agent_id: ctx.db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: message.to_string(),
            trigger_type: trigger_type.to_string(),
            cron_expr,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some(next_fire_at),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config,
            input_context: None,
            created_by_session: Some(ctx.session_id.to_string()),
        };

        let id = match ctx.db.create_task(task).await {
            Ok(id) => id,
            Err(e) if crate::db::is_unique_violation(&e) => {
                // Query for existing reminder to provide details
                let detail = if let Ok(tasks) = ctx.db.get_user_visible_tasks().await {
                    tasks
                        .iter()
                        .find(|t| {
                            t.label.eq_ignore_ascii_case(message) && t.action_type == "send_message"
                        })
                        .map(|t| {
                            let time_info = t
                                .next_fire_at
                                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                                .map(|dt| {
                                    format!(", next fire: {}", dt.format("%Y-%m-%d %H:%M UTC"))
                                })
                                .unwrap_or_default();
                            format!(
                                "A reminder '{}' already exists (id: {}{}). No duplicate created.",
                                t.label, t.id, time_info
                            )
                        })
                } else {
                    None
                };
                return Ok(ToolOutput::success(detail.unwrap_or_else(|| {
                    "A similar reminder already exists and is still active. No duplicate created."
                        .to_string()
                })));
            }
            Err(e) => return Err(e),
        };

        ctx.db
            .log_memory_event(
                ctx.session_id,
                "create_reminder",
                &format!("task:{id}"),
                None,
                &format!("{display} — {message}"),
                None,
            )
            .await?;

        let response = if trigger_type == "recurring" {
            format!("Periodic reminder {id} created (cron: {display}).")
        } else {
            format!("Reminder {id} scheduled for {display}.")
        };

        Ok(ToolOutput::success(response))
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

        let reminders = harness.db.get_user_visible_tasks().await.unwrap();
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

        // No fire_at and no cron_expr → error
        let result = tool
            .execute(serde_json::json!({"message": "no fire_at"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("fire_at"));

        // No message → error
        let result = tool
            .execute(serde_json::json!({"fire_at": "2099-12-31T23:59:59Z"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_create_recurring_reminder() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "cron_expr": "0 0 9 * * 1",
                    "message": "Weekly Monday standup"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Periodic reminder"));
        assert!(result.content.contains("0 0 9 * * 1"));

        let tasks = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].trigger_type, "recurring");
        assert_eq!(tasks[0].cron_expr.as_deref(), Some("0 0 9 * * 1"));
        assert_eq!(tasks[0].label, "Weekly Monday standup");
    }

    #[tokio::test]
    async fn test_create_recurring_reminder_invalid_cron() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "cron_expr": "not valid",
                    "message": "Bad cron"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid cron expression"));
    }

    #[tokio::test]
    async fn test_create_recurring_reminder_too_frequent() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // Every second — should be rejected
        let result = tool
            .execute(
                serde_json::json!({
                    "cron_expr": "* * * * * *",
                    "message": "Too frequent"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too frequently"));
    }

    #[tokio::test]
    async fn test_create_reminder_blocks_duplicate() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;
        let input = serde_json::json!({
            "fire_at": "2099-12-31T23:59:59Z",
            "message": "Year-end review"
        });

        // First creation succeeds
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("scheduled"));

        // Second creation with same label is blocked — with details
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already exists"));
        assert!(
            result.content.contains("Year-end review"),
            "should contain label"
        );
        assert!(result.content.contains("id:"), "should contain task id");
        assert!(
            result.content.contains("next fire:"),
            "should contain next fire time"
        );

        // Only one reminder in DB
        let reminders = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(reminders.len(), 1);
    }

    #[tokio::test]
    async fn test_create_reminder_allows_different_label() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        tool.execute(
            serde_json::json!({
                "fire_at": "2099-12-31T23:59:59Z",
                "message": "Year-end review"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "message": "Different reminder"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("scheduled"));

        let reminders = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(reminders.len(), 2);
    }

    #[tokio::test]
    async fn test_create_reminder_case_insensitive_duplicate() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        tool.execute(
            serde_json::json!({
                "fire_at": "2099-12-31T23:59:59Z",
                "message": "Year-end review"
            }),
            &ctx,
        )
        .await
        .unwrap();

        // Different case should still be blocked — with details from original
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "message": "YEAR-END REVIEW"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already exists"));
        assert!(result.content.contains("id:"), "should contain task id");

        let reminders = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(reminders.len(), 1);
    }

    #[tokio::test]
    async fn test_create_recurring_reminder_ignores_fire_at() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // When cron_expr is provided, fire_at is ignored
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "cron_expr": "0 0 18 * * *",
                    "message": "Daily evening check"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Periodic reminder"));

        let tasks = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(tasks[0].trigger_type, "recurring");
    }
}
