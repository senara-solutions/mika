use anyhow::Result;
use async_trait::async_trait;
use chrono::{NaiveDateTime, Utc};
use chrono_tz::Tz;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::NewTask;
use crate::task_engine::cron::{next_fire_from_cron, next_fire_from_cron_tz, parse_timezone};

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
                provide fire_at (local datetime) with timezone. For periodic reminders, \
                provide cron_expr (6-field cron expression with seconds field first) with timezone. \
                When timezone is provided, times are interpreted as local time and converted to UTC automatically. \
                Parse the user's natural language into the appropriate format \
                before calling this tool."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "fire_at": {
                        "type": "string",
                        "description": "When the reminder should fire. With timezone: local datetime e.g. '2026-04-02T09:00:00'. Without timezone: ISO 8601 UTC e.g. '2026-04-02T01:00:00Z'. Required for one-shot reminders, ignored for periodic."
                    },
                    "message": {
                        "type": "string",
                        "description": "The reminder message to deliver to the user"
                    },
                    "cron_expr": {
                        "type": "string",
                        "description": "6-field cron expression (seconds first) for periodic reminders. When timezone is provided, the schedule is in local time. Example: '0 0 9 * * 1' = every Monday at 9am, '0 0 18 * * *' = daily at 6pm, '0 0 9 * * 1-5' = weekdays at 9am."
                    },
                    "timezone": {
                        "type": "string",
                        "description": "IANA timezone name (e.g. 'Asia/Singapore', 'America/New_York'). When provided, fire_at and cron_expr are interpreted as local time in this timezone. Use the user's configured timezone."
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
        let timezone_input = input["timezone"].as_str().unwrap_or("");

        if message.is_empty() {
            return Ok(ToolOutput::error("'message' is required."));
        }
        if message.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'message' too long: {} characters (max: {MAX_INPUT_LEN})",
                message.len()
            )));
        }

        // Validate timezone if provided
        let validated_tz: Option<Tz> = if !timezone_input.is_empty() {
            match parse_timezone(timezone_input) {
                Ok(tz) => Some(tz),
                Err(_) => {
                    return Ok(ToolOutput::error(format!(
                        "Invalid timezone '{}'. Use IANA timezone names, e.g. 'Asia/Singapore', 'America/New_York', 'Europe/London'.",
                        timezone_input
                    )));
                }
            }
        } else {
            None
        };

        // Determine scheduling mode: periodic (cron) or one-shot (fire_at)
        let (trigger_type, cron_expr, next_fire_at, display, metadata) = if !cron_expr_input
            .is_empty()
        {
            // Periodic reminder
            if cron_expr_input.len() > 128 {
                return Ok(ToolOutput::error("'cron_expr' is too long."));
            }

            let now_str = crate::timestamp::now();
            let cron_result = if let Some(ref tz) = validated_tz {
                next_fire_from_cron_tz(cron_expr_input, &now_str, tz)
            } else {
                next_fire_from_cron(cron_expr_input, &now_str)
            };
            let next_fire = match cron_result {
                Ok(ts) => ts,
                Err(_) => {
                    return Ok(ToolOutput::error(
                        "Invalid cron expression. Use 6-field format (seconds first), \
                             e.g. '0 0 9 * * 1' for every Monday at 9am.",
                    ));
                }
            };

            // Reject cron expressions that fire more frequently than once per minute
            {
                let now_dt = crate::timestamp::parse(&now_str).unwrap_or_else(|_| Utc::now());
                let next_dt = crate::timestamp::parse(&next_fire).unwrap_or(now_dt);
                let interval = next_dt.signed_duration_since(now_dt).num_seconds();
                if interval < 60 {
                    let second_result = if let Some(ref tz) = validated_tz {
                        next_fire_from_cron_tz(cron_expr_input, &next_fire, tz)
                    } else {
                        next_fire_from_cron(cron_expr_input, &next_fire)
                    };
                    if let Ok(second_fire) = second_result {
                        let second_dt = crate::timestamp::parse(&second_fire).unwrap_or(next_dt);
                        let second_interval =
                            second_dt.signed_duration_since(next_dt).num_seconds();
                        if second_interval < 60 {
                            return Ok(ToolOutput::error(
                                "Cron expression fires too frequently. Minimum interval is 1 minute.",
                            ));
                        }
                    }
                }
            }

            // Store timezone in metadata for recurring reminders
            let meta = validated_tz
                .as_ref()
                .map(|_| serde_json::json!({"timezone": timezone_input}).to_string());

            let display = if validated_tz.is_some() {
                format!("periodic ({cron_expr_input}, {timezone_input})")
            } else {
                format!("periodic ({cron_expr_input})")
            };

            (
                "recurring",
                Some(cron_expr_input.to_string()),
                next_fire,
                display,
                meta,
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

            let (parsed_utc, display_time) = if let Some(tz) = validated_tz {
                // Timezone provided — interpret fire_at as local time
                match NaiveDateTime::parse_from_str(fire_at, "%Y-%m-%dT%H:%M:%S") {
                    Ok(naive) => {
                        let local_dt = match naive.and_local_timezone(tz) {
                            chrono::LocalResult::Single(dt) => dt,
                            chrono::LocalResult::Ambiguous(dt, _) => dt, // Use earlier offset
                            chrono::LocalResult::None => {
                                return Ok(ToolOutput::error(
                                    "The specified local time does not exist in this timezone (likely a DST gap). Try a time 1 hour later.",
                                ));
                            }
                        };
                        let utc_dt = local_dt.with_timezone(&Utc);
                        let display = format!(
                            "{} {} ({})",
                            naive.format("%Y-%m-%d %H:%M:%S"),
                            timezone_input,
                            utc_dt.format("%Y-%m-%d %H:%M:%S UTC")
                        );
                        (utc_dt, display)
                    }
                    Err(_) => {
                        // fire_at wasn't a naive datetime — check if it's offset-aware
                        match chrono::DateTime::parse_from_rfc3339(fire_at) {
                            Ok(_) => {
                                // Conflict: user provided both timezone and offset-aware fire_at
                                return Ok(ToolOutput::error(
                                    "Conflicting timezone info: 'fire_at' already includes a UTC offset (e.g. 'Z' or '+08:00'), \
                                     but 'timezone' was also provided. Either remove the 'timezone' parameter, \
                                     or use a local datetime format like '2026-04-02T09:00:00' with 'timezone'.",
                                ));
                            }
                            Err(_) => {
                                return Ok(ToolOutput::error(
                                    "Invalid datetime. With timezone, use local format like '2026-04-02T09:00:00'. \
                                     Without timezone, use ISO 8601 UTC like '2026-04-02T01:00:00Z'.",
                                ));
                            }
                        }
                    }
                }
            } else {
                // No timezone — expect UTC (backward compatible)
                match chrono::DateTime::parse_from_rfc3339(fire_at) {
                    Ok(dt) => {
                        let utc_dt = dt.with_timezone(&Utc);
                        let display = utc_dt.format("%Y-%m-%d %H:%M:%S UTC").to_string();
                        (utc_dt, display)
                    }
                    Err(_) => {
                        return Ok(ToolOutput::error(
                            "Invalid ISO 8601 datetime. Use format like '2026-02-25T15:00:00Z', or provide a timezone parameter to use local time.",
                        ));
                    }
                }
            };

            if parsed_utc <= Utc::now() {
                return Ok(ToolOutput::error("Reminder time must be in the future."));
            }

            let timestamp = crate::timestamp::format(&parsed_utc);
            ("time", None, timestamp, display_time, None)
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
            created_trace_id: Some(ctx.trace_id.to_string()),
            reference_url: None,
            source: None,
            metadata,
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
                                .as_ref()
                                .and_then(|ts| crate::timestamp::parse(ts).ok())
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
            .log_audit_event(
                ctx.session_id,
                "create_reminder",
                &format!("task:{id}"),
                None,
                Some(&*format!("{display} — {message}")),
                None,
                Some(ctx.trace_id),
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

    // --- Timezone parameter tests ---

    #[tokio::test]
    async fn test_create_reminder_with_timezone_local_time() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // 2099-12-31 09:00 Asia/Singapore = 2099-12-31T01:00:00Z
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T09:00:00",
                    "message": "Local time reminder",
                    "timezone": "Asia/Singapore"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("scheduled"));
        assert!(result.content.contains("Asia/Singapore"));

        let tasks = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        // Stored as UTC
        assert_eq!(
            tasks[0].next_fire_at.as_deref(),
            Some("2099-12-31T01:00:00Z")
        );
    }

    #[tokio::test]
    async fn test_create_reminder_utc_backward_compat() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // UTC with Z suffix, no timezone — backward compatible
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "message": "UTC reminder"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("scheduled"));
        assert!(result.content.contains("UTC"));
    }

    #[tokio::test]
    async fn test_create_reminder_invalid_timezone() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T09:00:00",
                    "message": "Bad tz",
                    "timezone": "Not/A/Timezone"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid timezone"));
    }

    #[tokio::test]
    async fn test_create_reminder_timezone_with_rfc3339_fallback() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // Providing both timezone and offset-aware fire_at should error
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T23:59:59Z",
                    "message": "Offset takes precedence",
                    "timezone": "Asia/Singapore"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.is_error,
            "expected error for conflicting timezone info"
        );
        assert!(
            result.content.contains("Conflicting timezone info"),
            "error should mention conflict: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_create_recurring_reminder_with_timezone() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // Recurring: "every day at 9am" in Asia/Singapore
        let result = tool
            .execute(
                serde_json::json!({
                    "cron_expr": "0 0 9 * * *",
                    "message": "Daily 9am SGT",
                    "timezone": "Asia/Singapore"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Periodic reminder"));
        assert!(result.content.contains("Asia/Singapore"));

        // Check metadata stores timezone
        let tasks = harness.db.get_user_visible_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        let meta = tasks[0].metadata.as_ref().expect("metadata should exist");
        let meta_json: serde_json::Value = serde_json::from_str(meta).unwrap();
        assert_eq!(meta_json["timezone"], "Asia/Singapore");
    }

    #[tokio::test]
    async fn test_create_reminder_naive_without_timezone_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateReminderTool;

        // Naive datetime without timezone should be rejected
        let result = tool
            .execute(
                serde_json::json!({
                    "fire_at": "2099-12-31T09:00:00",
                    "message": "No tz"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid ISO 8601"));
    }
}
