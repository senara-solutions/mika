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

/// Permitted status transitions. Terminal states (completed, cancelled) have no outbound edges.
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    (
        "pending",
        &["in_progress", "blocked", "completed", "cancelled"],
    ),
    ("in_progress", &["blocked", "completed", "cancelled"]),
    ("blocked", &["in_progress", "completed", "cancelled"]),
    ("completed", &[]),
    ("cancelled", &[]),
];

/// Check whether transitioning from `from` to `to` is permitted.
fn is_valid_transition(from: &str, to: &str) -> bool {
    VALID_TRANSITIONS
        .iter()
        .find(|(s, _)| *s == from)
        .map(|(_, targets)| targets.contains(&to))
        .unwrap_or(false)
}

/// Return the list of statuses reachable from `from`.
fn allowed_transitions(from: &str) -> &'static [&'static str] {
    VALID_TRANSITIONS
        .iter()
        .find(|(s, _)| *s == from)
        .map(|(_, targets)| *targets)
        .unwrap_or(&[])
}

/// Maximum metadata JSON size (10 KB).
const MAX_METADATA_LEN: usize = 10_240;

pub struct UpdateWorkItemStatusTool;

#[async_trait]
impl Tool for UpdateWorkItemStatusTool {
    fn name(&self) -> &str {
        "update_work_item_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_work_item_status".to_string(),
            description: "Update the status of a work item (manual task). \
                Only works on manual work items, not system tasks (reminders, callbacks, etc.). \
                Transitions are validated: pending can go to any status; in_progress can go to \
                blocked/completed/cancelled; blocked can go to in_progress/completed/cancelled. \
                Completed and cancelled are terminal — cannot be changed. \
                Every transition is logged as an audit event. \
                Optionally attach or merge structured metadata (JSON object) on the work item."
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
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional structured metadata to merge into the work item. \
                            Shallow-merged at the top level AND one level deep for object-valued fields \
                            (e.g. fields under `claude_pilot.*` from a prior callback are preserved when \
                            you write a new key under `claude_pilot`). Max 10 KB."
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
        let metadata_input = input.get("metadata").filter(|v| !v.is_null());

        // Validate inputs
        if task_id.is_empty() {
            return Ok(ToolOutput::error("'task_id' is required."));
        }
        if let Err(e) = super::validate_uuid("task_id", task_id) {
            return Ok(e);
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

        // Validate metadata if provided
        if let Some(meta) = metadata_input {
            if !meta.is_object() {
                return Ok(ToolOutput::error(
                    "'metadata' must be a JSON object, not an array or scalar.",
                ));
            }
            let meta_str = serde_json::to_string(meta)?;
            if meta_str.len() > MAX_METADATA_LEN {
                return Ok(ToolOutput::error(format!(
                    "'metadata' is too large ({} bytes). Maximum is {} bytes.",
                    meta_str.len(),
                    MAX_METADATA_LEN
                )));
            }
        }

        // Look up the work item to get the current status for transition validation
        let task = match ctx.db.get_task(task_id).await? {
            Some(t) if t.trigger_type == "manual" => t,
            Some(_) | None => {
                return Ok(ToolOutput::error(format!(
                    "Work item '{task_id}' not found. Only manual (work item) tasks can be updated with this tool."
                )));
            }
        };

        let old_status = task.status.clone();

        // Same-status is a no-op (skip transition validation)
        if old_status == status {
            if let Some(new_meta) = metadata_input {
                merge_and_persist_metadata(task_id, new_meta, ctx).await?;
            }
            let mut response =
                format!("Work item {task_id} is already '{status}'. No status change.");
            if metadata_input.is_some() {
                response.push_str(" Metadata updated.");
            }
            return Ok(ToolOutput::success(response));
        }

        // Validate the transition against the state machine
        if !is_valid_transition(&old_status, status) {
            let allowed = allowed_transitions(&old_status);
            return if allowed.is_empty() {
                Ok(ToolOutput::error(format!(
                    "Cannot transition from '{old_status}' to '{status}'. \
                     '{old_status}' is a terminal state — completed and cancelled work items cannot be changed."
                )))
            } else {
                Ok(ToolOutput::error(format!(
                    "Cannot transition from '{old_status}' to '{status}'. \
                     Valid transitions from '{old_status}': {}.",
                    allowed.join(", ")
                )))
            };
        }

        // Perform the status update
        ctx.db.update_manual_task_status(task_id, status).await?;

        // Merge and persist metadata if provided
        if let Some(new_meta) = metadata_input {
            merge_and_persist_metadata(task_id, new_meta, ctx).await?;
        }

        // Log audit event
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "update_work_item_status",
                &format!("task:{task_id}"),
                Some(&old_status),
                Some(status),
                note,
                Some(ctx.trace_id),
            )
            .await?;

        let mut response = format!("Work item {task_id}: {old_status} → {status}");
        if let Some(n) = note {
            response.push_str(&format!("\nNote: {n}"));
        }
        if metadata_input.is_some() {
            response.push_str("\nMetadata updated.");
        }

        Ok(ToolOutput::success(response))
    }
}

/// Shallow-merge `new_meta` into the work item's existing metadata and persist.
async fn merge_and_persist_metadata(
    task_id: &str,
    new_meta: &Value,
    ctx: &ToolContext<'_>,
) -> Result<()> {
    let merged = if let Ok(Some(task)) = ctx.db.get_task_unscoped(task_id).await {
        if let Some(existing_str) = &task.metadata {
            if let Ok(mut existing) = serde_json::from_str::<Value>(existing_str) {
                crate::work_item_metadata::merge_metadata(&mut existing, new_meta);
                existing
            } else {
                new_meta.clone()
            }
        } else {
            new_meta.clone()
        }
    } else {
        new_meta.clone()
    };

    let merged_str = serde_json::to_string(&merged)?;
    ctx.db
        .update_work_item_metadata(task_id, &merged_str)
        .await?;
    Ok(())
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
                metadata: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_update_status_basic() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Test work item").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

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
        let tool = UpdateWorkItemStatusTool;

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
        let tool = UpdateWorkItemStatusTool;

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
        let tool = UpdateWorkItemStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": "00000000-0000-0000-0000-000000000000", "status": "invalid"}),
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
        let tool = UpdateWorkItemStatusTool;

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
                metadata: None,
            })
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

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
        let tool = UpdateWorkItemStatusTool;

        let result = tool
            .execute(serde_json::json!({"status": "completed"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'task_id' is required"));

        let result = tool
            .execute(
                serde_json::json!({"task_id": "00000000-0000-0000-0000-000000000000"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'status' is required"));
    }

    #[tokio::test]
    async fn test_update_status_with_metadata() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Meta item").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"claude_pilot": {"branch": "feat/test", "repo": "mika"}}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("pending → in_progress"));
        assert!(result.content.contains("Metadata updated"));
    }

    #[tokio::test]
    async fn test_update_status_metadata_merge() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Merge meta").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"claude_pilot": {"branch": "feat/test"}}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"extra": "data"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Metadata updated"));
    }

    #[tokio::test]
    async fn test_metadata_inner_object_fields_preserved_across_updates() {
        // Issue #489: engine writes claude_pilot.{cost_usd,duration_ms,session_id,turns},
        // agent enriches with claude_pilot.{pr_url,branch} — all six fields must survive.
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "489 repro").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        // Turn 1: engine-injected claude_pilot fields
        let r1 = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {
                        "claude_pilot": {
                            "cost_usd": "6.465109",
                            "duration_ms": 1230514,
                            "session_id": "d29d3852",
                            "turns": 102
                        }
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!r1.is_error, "{}", r1.content);

        // Turn 2: agent enrichment — must NOT clobber turn 1 fields
        let r2 = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {
                        "claude_pilot": {
                            "pr_url": "https://github.com/senara-solutions/mika/pull/19",
                            "branch": "fix/489/x"
                        }
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!r2.is_error, "{}", r2.content);

        // Read back and assert all six fields are present.
        let task = harness.db.get_task_unscoped(&id).await.unwrap().unwrap();
        let meta_str = task.metadata.expect("metadata should exist");
        let meta: serde_json::Value = serde_json::from_str(&meta_str).unwrap();
        let cp = meta
            .get("claude_pilot")
            .and_then(|v| v.as_object())
            .expect("claude_pilot object");

        assert_eq!(cp.get("cost_usd").unwrap(), "6.465109");
        assert_eq!(cp.get("duration_ms").unwrap(), 1230514);
        assert_eq!(cp.get("session_id").unwrap(), "d29d3852");
        assert_eq!(cp.get("turns").unwrap(), 102);
        assert_eq!(
            cp.get("pr_url").unwrap(),
            "https://github.com/senara-solutions/mika/pull/19"
        );
        assert_eq!(cp.get("branch").unwrap(), "fix/489/x");
    }

    #[tokio::test]
    async fn test_metadata_top_level_keys_preserved_across_updates() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "top-level merge").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        tool.execute(
            serde_json::json!({
                "task_id": id,
                "status": "in_progress",
                "metadata": {"claude_pilot": {"cost_usd": "1.00"}}
            }),
            &ctx,
        )
        .await
        .unwrap();

        tool.execute(
            serde_json::json!({
                "task_id": id,
                "status": "in_progress",
                "metadata": {"github": {"pr": 19}}
            }),
            &ctx,
        )
        .await
        .unwrap();

        let task = harness.db.get_task_unscoped(&id).await.unwrap().unwrap();
        let meta: serde_json::Value = serde_json::from_str(&task.metadata.unwrap()).unwrap();
        assert_eq!(meta["claude_pilot"]["cost_usd"], "1.00");
        assert_eq!(meta["github"]["pr"], 19);
    }

    #[tokio::test]
    async fn test_update_status_rejects_non_object_metadata() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Bad meta").await;
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": "not an object"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("JSON object"));

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": [1, 2, 3]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("JSON object"));
    }

    #[tokio::test]
    async fn test_valid_forward_transitions() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        // pending → in_progress
        let id = create_work_item(&harness, "Forward 1").await;
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "in_progress"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "pending→in_progress failed: {}",
            result.content
        );

        // in_progress → blocked
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "blocked"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "in_progress→blocked failed: {}",
            result.content
        );

        // blocked → completed
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "completed"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "blocked→completed failed: {}",
            result.content
        );

        // pending → completed (skip in_progress)
        let id2 = create_work_item(&harness, "Forward 2").await;
        let result = tool
            .execute(
                serde_json::json!({"task_id": id2, "status": "completed"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "pending→completed failed: {}",
            result.content
        );

        // pending → cancelled
        let id3 = create_work_item(&harness, "Forward 3").await;
        let result = tool
            .execute(
                serde_json::json!({"task_id": id3, "status": "cancelled"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "pending→cancelled failed: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_blocked_to_in_progress_allowed() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        let id = create_work_item(&harness, "Unblock item").await;

        // pending → blocked
        tool.execute(
            serde_json::json!({"task_id": id, "status": "blocked"}),
            &ctx,
        )
        .await
        .unwrap();

        // blocked → in_progress (un-block)
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "in_progress"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "blocked→in_progress failed: {}",
            result.content
        );
        assert!(result.content.contains("blocked → in_progress"));
    }

    #[tokio::test]
    async fn test_rejected_backward_transition() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        let id = create_work_item(&harness, "No regress").await;

        // pending → in_progress
        tool.execute(
            serde_json::json!({"task_id": id, "status": "in_progress"}),
            &ctx,
        )
        .await
        .unwrap();

        // in_progress → pending (should be rejected)
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "pending"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error, "in_progress→pending should be rejected");
        assert!(result.content.contains("Cannot transition"));
        assert!(
            result
                .content
                .contains("Valid transitions from 'in_progress'")
        );
    }

    #[tokio::test]
    async fn test_terminal_state_cannot_transition() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateWorkItemStatusTool;

        // Test completed → anything
        let id = create_work_item(&harness, "Completed item").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "completed"}),
            &ctx,
        )
        .await
        .unwrap();

        for target in &["pending", "in_progress", "blocked", "cancelled"] {
            let result = tool
                .execute(serde_json::json!({"task_id": id, "status": target}), &ctx)
                .await
                .unwrap();
            assert!(
                result.is_error,
                "completed→{target} should be rejected, got: {}",
                result.content
            );
            assert!(
                result.content.contains("terminal state"),
                "expected terminal state message for completed→{target}, got: {}",
                result.content
            );
        }

        // Test cancelled → anything
        let id2 = create_work_item(&harness, "Cancelled item").await;
        tool.execute(
            serde_json::json!({"task_id": id2, "status": "cancelled"}),
            &ctx,
        )
        .await
        .unwrap();

        for target in &["pending", "in_progress", "blocked", "completed"] {
            let result = tool
                .execute(serde_json::json!({"task_id": id2, "status": target}), &ctx)
                .await
                .unwrap();
            assert!(
                result.is_error,
                "cancelled→{target} should be rejected, got: {}",
                result.content
            );
            assert!(
                result.content.contains("terminal state"),
                "expected terminal state message for cancelled→{target}, got: {}",
                result.content
            );
        }
    }

    #[test]
    fn test_transition_helpers() {
        // is_valid_transition
        assert!(is_valid_transition("pending", "in_progress"));
        assert!(is_valid_transition("pending", "completed"));
        assert!(is_valid_transition("pending", "cancelled"));
        assert!(is_valid_transition("blocked", "in_progress"));
        assert!(!is_valid_transition("in_progress", "pending"));
        assert!(!is_valid_transition("completed", "pending"));
        assert!(!is_valid_transition("cancelled", "in_progress"));
        assert!(!is_valid_transition("unknown", "pending"));

        // allowed_transitions
        assert_eq!(
            allowed_transitions("pending"),
            &["in_progress", "blocked", "completed", "cancelled"]
        );
        assert_eq!(
            allowed_transitions("in_progress"),
            &["blocked", "completed", "cancelled"]
        );
        assert_eq!(
            allowed_transitions("blocked"),
            &["in_progress", "completed", "cancelled"]
        );
        assert!(allowed_transitions("completed").is_empty());
        assert!(allowed_transitions("cancelled").is_empty());
        assert!(allowed_transitions("unknown").is_empty());
    }
}
