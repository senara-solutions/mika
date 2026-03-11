use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::task_engine::trigger_type;

const MAX_RESULT_LEN: usize = 100_000;

pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn name(&self) -> &str {
        "complete_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "complete_task".to_string(),
            description: "Mark a callback task as completed and provide its result. Only works for tasks with trigger_type=callback that are in pending or in_progress status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the callback task to complete."
                    },
                    "result": {
                        "type": "string",
                        "description": "The result or output of the completed work (required, max 100KB)."
                    }
                },
                "required": ["id", "result"]
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

        let result = input["result"].as_str().unwrap_or("").trim();
        if result.is_empty() {
            return Ok(ToolOutput::error("'result' is required."));
        }
        if result.len() > MAX_RESULT_LEN {
            return Ok(ToolOutput::error(format!(
                "'result' exceeds maximum length of {} characters.",
                MAX_RESULT_LEN
            )));
        }

        // Load and validate the task
        let task = match ctx.db.get_task(id).await? {
            Some(t) => t,
            None => return Ok(ToolOutput::error(format!("Task '{}' not found.", id))),
        };

        if task.trigger_type != trigger_type::CALLBACK {
            return Ok(ToolOutput::error(format!(
                "Task '{}' has trigger_type '{}', not 'callback'. Only callback tasks can be completed with this tool.",
                id, task.trigger_type
            )));
        }

        if !matches!(task.status.as_str(), "pending" | "in_progress") {
            return Ok(ToolOutput::error(format!(
                "Task '{}' has status '{}' and cannot be completed.",
                id, task.status
            )));
        }

        // Mark the task as completed (returns false if already in terminal state)
        if !ctx.db.update_task_completed(id, Some(result)).await? {
            return Ok(ToolOutput::error(format!(
                "Task '{}' could not be completed: already in a terminal state.",
                id
            )));
        }

        // Audit log
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "complete_task",
                &format!("task:{id}"),
                None,
                &format!("Completed callback task: {} ({})", task.label, id),
                None,
                Some(ctx.trace_id),
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Task {} ('{}') marked as completed.",
            id, task.label
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    async fn add_callback_task(harness: &TestHarness) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "test callback".to_string(),
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
                created_trace_id: None,
                reference_url: None,
                source: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_complete_task_success() {
        let harness = TestHarness::new();
        let task_id = add_callback_task(&harness).await;
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": "analysis done"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(!output.is_error, "got error: {}", output.content);
        assert!(output.content.contains(&task_id));
    }

    #[tokio::test]
    async fn test_complete_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input =
            serde_json::json!({"id": "00000000-0000-0000-0000-000000000000", "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_complete_task_wrong_trigger_type() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // Create a time-triggered task via create_task tool
        let create_input = serde_json::json!({
            "label": "time reminder",
            "trigger_type": "time",
            "action_type": "send_message",
            "action_config": "{\"text\":\"hello\"}",
            "fire_at": "2099-01-01T00:00:00Z"
        });
        let create_output = crate::tools::create_task::CreateTaskTool
            .execute(create_input, &ctx)
            .await
            .unwrap();
        assert!(
            !create_output.is_error,
            "setup failed: {}",
            create_output.content
        );
        // Extract UUID from "Task <uuid> created ..."
        let task_id = create_output
            .content
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();

        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("callback"));
    }

    #[tokio::test]
    async fn test_complete_task_empty_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": "", "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_complete_task_empty_result() {
        let harness = TestHarness::new();
        let task_id = add_callback_task(&harness).await;
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let input = serde_json::json!({"id": task_id, "result": ""});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("required"));
    }

    #[tokio::test]
    async fn test_complete_task_id_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CompleteTaskTool;
        let long_id = "a".repeat(37);
        let input = serde_json::json!({"id": long_id, "result": "result"});
        let output = tool.execute(input, &ctx).await.unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("UUID"));
    }
}
