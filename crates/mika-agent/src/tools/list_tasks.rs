use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_ts;

pub struct ListTasksTool;

#[async_trait]
impl Tool for ListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "List all active tasks including reminders, recurring heartbeat, reflection, and other scheduled tasks. Optionally filter by status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Optional status filter. One of: 'pending', 'recurring_active', 'in_progress'. Omit to show all active tasks (pending and recurring_active).",
                        "enum": ["pending", "recurring_active", "in_progress"]
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let statuses: Vec<String> = if let Some(s) = input["status"].as_str() {
            vec![s.to_string()]
        } else {
            vec!["pending".to_string(), "recurring_active".to_string()]
        };

        let tasks = ctx.db.get_tasks_by_status(statuses).await?;

        if tasks.is_empty() {
            return Ok(ToolOutput::success("No active tasks."));
        }

        let mut output = String::from("Active tasks:\n");
        for t in &tasks {
            let fire_at = t
                .next_fire_at
                .as_ref()
                .map(|s| format_ts(s))
                .unwrap_or_else(|| "no schedule".to_string());
            let timeout = t
                .timeout_at
                .as_ref()
                .map(|s| format_ts(s))
                .unwrap_or_else(|| "none".to_string());
            output.push_str(&format!(
                "- {} [{}/{}] {} ({}) — next: {} | timeout: {}\n",
                t.id, t.status, t.trigger_type, t.label, t.action_type, fire_at, timeout
            ));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_task(harness: &TestHarness, label: &str, action_type: &str) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: label.to_string(),
                trigger_type: "time".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
                timeout_at: None,
                action_type: action_type.to_string(),
                action_config: serde_json::json!({"text": label}).to_string(),
                input_context: None,
                created_by_session: None,
                created_trace_id: None,
                reference_url: None,
                source: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_list_tasks_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No active tasks"));
    }

    #[tokio::test]
    async fn test_list_tasks_shows_all_action_types() {
        let harness = TestHarness::new();
        add_task(&harness, "Morning reminder", "send_message").await;
        add_task(&harness, "Daily reflection", "run_skill").await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Morning reminder"));
        assert!(result.content.contains("Daily reflection"));
        assert!(result.content.contains("send_message"));
        assert!(result.content.contains("run_skill"));
    }

    #[tokio::test]
    async fn test_list_tasks_status_filter() {
        let harness = TestHarness::new();
        add_task(&harness, "Pending task", "send_message").await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        // Filter for pending only
        let result = tool
            .execute(serde_json::json!({"status": "pending"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Pending task"));

        // Filter for recurring_active — none created as recurring here
        let result = tool
            .execute(serde_json::json!({"status": "recurring_active"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No active tasks"));
    }

    #[tokio::test]
    async fn test_list_tasks_shows_full_id_and_schedule() {
        let harness = TestHarness::new();
        let id = add_task(&harness, "Check-in", "send_message").await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        // Full UUID and scheduling info should be present
        assert!(result.content.contains("Check-in"));
        assert!(result.content.contains("pending"));
        // Full UUID should appear in output (not just 8 chars)
        assert!(result.content.contains(&id));
        // trigger_type should appear
        assert!(result.content.contains("time"));
        // timeout should appear as "none" when not set
        assert!(result.content.contains("timeout: none"));
    }
}
