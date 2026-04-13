use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn name(&self) -> &str {
        "cancel_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cancel_task".to_string(),
            description:
                "Cancel any pending or in-progress task by its full UUID (from list_tasks or create_task). \
                Works for any task type: reminders, callback tasks, recurring tasks, etc. \
                If the task has a running process (e.g., claude-pilot), the process is killed."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The full UUID of the task to cancel"
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
        if let Err(e) = super::validate_uuid("id", id) {
            return Ok(e);
        }

        let outcome = crate::task_engine::process_kill::cancel_task_and_kill(ctx.db, id).await?;

        let Some(outcome) = outcome else {
            return Ok(ToolOutput::error(format!(
                "Task {id} not found or not in cancellable status."
            )));
        };

        // Build kill status message
        let kill_msg = match (outcome.process_killed, outcome.pid) {
            (Some(true), Some(pid)) => format!(" Process (PID {pid}) terminated."),
            (Some(false), Some(pid)) => {
                format!(" Warning: process (PID {pid}) may still be running.")
            }
            _ => String::new(),
        };

        ctx.db
            .log_audit_event(
                ctx.session_id,
                "cancel_task",
                &format!("task:{id}"),
                None,
                Some("cancelled"),
                None,
                Some(ctx.trace_id),
            )
            .await?;

        Ok(ToolOutput::success(format!(
            "Task {id} (\"{}\") has been cancelled.{kill_msg}",
            outcome.label
        )))
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
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_cancel_task_success() {
        let harness = TestHarness::new();
        let id = add_callback_task(&harness, "Analyze codebase").await;

        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool
            .execute(serde_json::json!({"id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("cancelled"));
        assert!(result.content.contains("Analyze codebase"));
    }

    #[tokio::test]
    async fn test_cancel_task_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelTaskTool;

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
    async fn test_cancel_task_missing_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CancelTaskTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }
}
