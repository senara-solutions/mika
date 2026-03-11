use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

const VALID_STATUSES: &[&str] = &[
    "pending",
    "in_progress",
    "blocked",
    "completed",
    "cancelled",
];

pub struct UpdateTaskStatusTool;

#[async_trait]
impl Tool for UpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Update the status of a work item (manual task). \
                Free transitions allowed — any status can move to any other status. \
                Every transition is logged as an audit event."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The UUID of the work item to update"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "blocked", "completed", "cancelled"],
                        "description": "The new status for the work item"
                    },
                    "note": {
                        "type": "string",
                        "description": "Optional reason for the status change"
                    }
                },
                "required": ["task_id", "status"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let task_id = input["task_id"].as_str().unwrap_or("").trim();
        let status = input["status"].as_str().unwrap_or("").trim();
        let note = input["note"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        // Validate inputs
        if task_id.is_empty() {
            return Ok(ToolOutput::error("'task_id' is required."));
        }
        if task_id.len() > 36 {
            return Ok(ToolOutput::error(
                "'task_id' must be a valid UUID (36 characters).",
            ));
        }
        if status.is_empty() {
            return Ok(ToolOutput::error("'status' is required."));
        }
        if !VALID_STATUSES.contains(&status) {
            return Ok(ToolOutput::error(format!(
                "Invalid status '{}'. Must be one of: {}",
                status,
                VALID_STATUSES.join(", ")
            )));
        }
        if let Some(n) = note
            && n.len() > MAX_INPUT_LEN
        {
            return Ok(ToolOutput::error("'note' is too long."));
        }

        // Update the task status (only works for manual/work item tasks)
        let old_status = match ctx.db.update_manual_task_status(task_id, status).await? {
            Some(old) => old,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Work item '{task_id}' not found. Only manual (work item) tasks can be updated with this tool."
                )));
            }
        };

        if old_status == status {
            return Ok(ToolOutput::success(format!(
                "Work item {task_id} is already '{status}'. No change."
            )));
        }

        // Log audit event
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "update_task_status",
                &format!("task:{task_id}"),
                Some(&old_status),
                status,
                note,
                Some(ctx.trace_id),
            )
            .await?;

        let mut response = format!("Work item {task_id}: {old_status} → {status}");
        if let Some(n) = note {
            response.push_str(&format!("\nNote: {n}"));
        }

        Ok(ToolOutput::success(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::task_engine::types::{action_type, trigger_type};
    use crate::test_utils::test_helpers::TestHarness;

    async fn create_work_item(harness: &TestHarness, label: &str) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: label.to_string(),
                trigger_type: trigger_type::MANUAL.to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: action_type::NONE.to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: Some("test-session".to_string()),
                created_trace_id: None,
                reference_url: None,
                source: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_update_status_basic() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Test work item").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "in_progress"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("pending → in_progress"));
    }

    #[tokio::test]
    async fn test_update_status_with_note() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Noted item").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "blocked",
                    "note": "Waiting for API access"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("pending → blocked"));
        assert!(result.content.contains("Waiting for API access"));
    }

    #[tokio::test]
    async fn test_update_status_same_status() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Same status").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "pending"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already 'pending'"));
    }

    #[tokio::test]
    async fn test_update_status_invalid_status() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": "some-id", "status": "invalid"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid status"));
    }

    #[tokio::test]
    async fn test_update_status_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": "00000000-0000-0000-0000-000000000000",
                    "status": "completed"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_status_rejects_non_manual_task() {
        let harness = TestHarness::new();
        // Create a callback task (not manual)
        let id = harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Callback task".to_string(),
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
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "completed"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_status_empty_fields() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(serde_json::json!({"status": "completed"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'task_id' is required"));

        let result = tool
            .execute(serde_json::json!({"task_id": "abc"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'status' is required"));
    }
}
