use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_ts;
use crate::task_engine::cron::{extract_timezone_from_metadata, parse_timezone};

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
            // Extract timezone from metadata if available
            let tz_name = extract_timezone_from_metadata(t.metadata.as_deref());

            let fire_at_display = t
                .next_fire_at
                .as_ref()
                .map(|s| {
                    // If timezone is available, show local time
                    if let Some(ref tz_str) = tz_name
                        && let Ok(tz) = parse_timezone(tz_str)
                        && let Ok(utc_dt) = crate::timestamp::parse(s)
                    {
                        let local_dt = utc_dt.with_timezone(&tz);
                        return format!("{} {}", local_dt.format("%Y-%m-%d %H:%M:%S"), tz_str);
                    }
                    // Fallback to UTC display
                    format!("{} UTC", format_ts(s))
                })
                .unwrap_or_else(|| "unknown".to_string());

            let action_badge = if t.action_type == "resume_agent" {
                " [auto]"
            } else {
                ""
            };

            if let Some(ref cron) = t.cron_expr {
                let tz_label = tz_name
                    .as_deref()
                    .map(|tz| format!(", tz: {tz}"))
                    .unwrap_or_default();
                output.push_str(&format!(
                    "- {}: \"{}\" next: {} (recurring: {}{}){}\n",
                    t.id, t.label, fire_at_display, cron, tz_label, action_badge
                ));
            } else {
                output.push_str(&format!(
                    "- {}: \"{}\" at {}{}\n",
                    t.id, t.label, fire_at_display, action_badge
                ));
            }
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_reminder(harness: &TestHarness, fire_at: &str, message: &str) -> String {
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
                next_fire_at: Some(fire_at.to_string()),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: serde_json::json!({"text": message}).to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
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
        add_reminder(&harness, "2099-01-01T00:00:00Z", "Meeting prep").await;
        // 2099-06-15T12:00:00Z
        add_reminder(&harness, "2099-06-15T12:00:00Z", "Birthday party").await;

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Meeting prep"));
        assert!(result.content.contains("Birthday party"));
    }

    #[tokio::test]
    async fn test_list_reminders_shows_recurrence() {
        let harness = TestHarness::new();
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Daily standup".to_string(),
                trigger_type: "recurring".to_string(),
                cron_expr: Some("0 0 9 * * *".to_string()),
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some("2099-01-01T00:00:00Z".to_string()),
                timeout_at: None,
                action_type: "send_message".to_string(),
                action_config: serde_json::json!({"text": "Daily standup"}).to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Daily standup"));
        assert!(result.content.contains("recurring: 0 0 9 * * *"));
    }

    #[tokio::test]
    async fn test_list_reminders_shows_action_badge() {
        let harness = TestHarness::new();
        // Create a resume_agent reminder
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Check CI and merge".to_string(),
                trigger_type: "time".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some("2099-01-01T00:00:00Z".to_string()),
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: serde_json::json!({"text": "Check CI and merge"}).to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap();

        // Also create a normal send_message reminder
        add_reminder(&harness, "2099-06-15T12:00:00Z", "Normal reminder").await;

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Check CI and merge"));
        assert!(result.content.contains("[auto]"));
        // Normal reminder should not have action badge
        assert!(result.content.contains("Normal reminder"));
        // Count occurrences of [auto] — should be exactly 1
        assert_eq!(result.content.matches("[auto]").count(), 1);
    }

    #[tokio::test]
    async fn test_list_reminders_excludes_cancelled() {
        let harness = TestHarness::new();
        let id = add_reminder(&harness, "2099-01-01T00:00:00Z", "Cancelled one").await;
        harness.db.cancel_task(&id).await.unwrap();
        add_reminder(&harness, "2099-06-15T12:00:00Z", "Active one").await;

        let ctx = harness.ctx();
        let tool = ListRemindersTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.content.contains("Cancelled one"));
        assert!(result.content.contains("Active one"));
    }
}
