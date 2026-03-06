use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_unix_ts;

pub struct GetTaskTool;

#[async_trait]
impl Tool for GetTaskTool {
    fn name(&self) -> &str {
        "get_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_task".to_string(),
            description: "Look up a task by its full UUID and return all task fields including status, trigger type, action type, schedule, and result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the task to look up."
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
        if id.len() > 36 {
            return Ok(ToolOutput::error(
                "'id' must be a valid task UUID (36 characters).",
            ));
        }

        let task = match ctx.db.get_task(id).await? {
            Some(t) => t,
            None => return Ok(ToolOutput::error(format!("Task '{}' not found.", id))),
        };

        let next_fire = task
            .next_fire_at
            .map(format_unix_ts)
            .unwrap_or_else(|| "N/A".to_string());
        let timeout = task
            .timeout_at
            .map(format_unix_ts)
            .unwrap_or_else(|| "none".to_string());
        let result = task.result.as_deref().unwrap_or("none");
        let created = format_unix_ts(task.created_at);

        let output = format!(
            "Task: {}\nLabel: {}\nStatus: {}\nTrigger: {}\nAction: {}\nCreated: {}\nNext fire: {}\nTimeout: {}\nResult: {}",
            task.id,
            task.label,
            task.status,
            task.trigger_type,
            task.action_type,
            created,
            next_fire,
            timeout,
            result,
        );

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_callback_task(harness: &TestHarness, label: &str) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: label.to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_get_task_success() {
        let harness = TestHarness::new();
        let id = add_callback_task(&harness, "My callback task").await;
        let ctx = harness.ctx();
        let tool = GetTaskTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains(&id));
        assert!(result.content.contains("My callback task"));
        assert!(result.content.contains("callback"));
        assert!(result.content.contains("resume_agent"));
        assert!(result.content.contains("pending"));
    }

    #[tokio::test]
    async fn test_get_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTaskTool;

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
    async fn test_get_task_empty_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTaskTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }

    #[tokio::test]
    async fn test_get_task_id_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetTaskTool;

        let long_id = "x".repeat(37);
        let result = tool
            .execute(serde_json::json!({"id": long_id}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("UUID"));
    }
}
