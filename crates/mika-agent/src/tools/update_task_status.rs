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

/// Permitted status transitions. `completed` is terminal (no outbound edges).
/// `cancelled` can return to `in_progress` (cancel-and-retry, mika#856).
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    (
        "pending",
        &["in_progress", "blocked", "completed", "cancelled"],
    ),
    ("in_progress", &["blocked", "completed", "cancelled"]),
    ("blocked", &["in_progress", "completed", "cancelled"]),
    ("completed", &[]),
    ("cancelled", &["in_progress"]),
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

pub struct UpdateTaskStatusTool;

#[async_trait]
impl Tool for UpdateTaskStatusTool {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "Update the status of a task (manual task). \
                Only works on manual tasks, not system tasks (reminders, callbacks, etc.). \
                Transitions are validated: pending can go to any status; in_progress can go to \
                blocked/completed/cancelled; blocked can go to in_progress/completed/cancelled. \
                Cancelled can return to in_progress (cancel-and-retry — reuses the same task \
                row, mika#856). Completed is terminal. For completed tasks, status \
                cannot be changed, but metadata can still be attached by passing the metadata \
                field (the status field is ignored in that case and the call succeeds). \
                Every transition is logged as an audit event. \
                Optionally attach or merge structured metadata (JSON object) on the task."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The UUID of the task to update"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "blocked", "completed", "cancelled"],
                        "description": "The new status for the task"
                    },
                    "note": {
                        "type": "string",
                        "description": "Optional reason for the status change"
                    },
                    "metadata": {
                        "type": "object",
                        "description": "Optional structured metadata to merge into the task. \
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

        // Format + existence + agent-scope validation in one call
        let task = match super::validate_task_exists(ctx.db, "task_id", task_id).await {
            Ok(t) if t.trigger_type == "manual" => t,
            Ok(t) => {
                return Ok(ToolOutput::error(format!(
                    "Task '{task_id}' exists but is not a manual task (trigger_type='{}'). \
                     This tool only operates on tasks created with create_task.",
                    t.trigger_type
                )));
            }
            Err(e) => return Ok(e),
        };

        let old_status = task.status.clone();

        // Phantom retry guard (#579): reject retry-semantic metadata writes when the
        // task has an active callback child task (i.e., a dispatch is still running).
        // This prevents the LLM from fabricating pipeline failures and polluting retry
        // budget before any callback has returned. Only fires when metadata contains
        // retry-related keys — non-retry metadata writes are unaffected.
        if let Some(meta) = metadata_input
            && has_retry_semantic_keys(meta)
            && let Ok(children) = ctx.db.get_child_tasks(task_id).await
        {
            let active_callback = children.iter().find(|c| {
                c.trigger_type == "callback"
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = active_callback {
                return Ok(ToolOutput::error(
                    serde_json::json!({
                        "error": "retry_metadata_rejected_active_dispatch",
                        "task_id": task_id,
                        "active_child_id": child.id,
                        "active_child_status": child.status,
                        "reason": format!(
                            "Cannot write retry-related metadata while a dispatch is \
                             still running (callback task '{}' is '{}'). Retry decisions \
                             must wait for the callback to complete. If you are seeing \
                             this error, the pipeline has NOT failed yet — do not retry.",
                            child.id, child.status
                        )
                    })
                    .to_string(),
                ));
            }
            // If get_child_tasks fails (caught by let-else above), allow the write
            // (fail-open) — the dispatch readiness guard is the primary defense.
        }

        // Same-status is a no-op (skip transition validation)
        if old_status == status {
            if let Some(new_meta) = metadata_input {
                merge_and_persist_metadata(task_id, new_meta, ctx).await?;
            }
            let mut response = format!("Task {task_id} is already '{status}'. No status change.");
            if metadata_input.is_some() {
                response.push_str(" Metadata updated.");
            }
            return Ok(ToolOutput::success(response));
        }

        // Validate the transition against the state machine
        if !is_valid_transition(&old_status, status) {
            let allowed = allowed_transitions(&old_status);

            // Terminal-state metadata fallback (#617): when the task is in a terminal
            // state and the caller provided metadata, apply the metadata and skip the
            // status change instead of rejecting the entire call. This prevents
            // late-arriving callbacks from losing metadata on tasks that were already
            // completed by a faster structural handler.
            if allowed.is_empty() {
                if let Some(new_meta) = metadata_input {
                    merge_and_persist_metadata(task_id, new_meta, ctx).await?;
                    return Ok(ToolOutput::success(format!(
                        "Status unchanged ('{old_status}' is terminal). Metadata updated."
                    )));
                }
                return Ok(ToolOutput::error(format!(
                    "Cannot transition from '{old_status}' to '{status}'. \
                     '{old_status}' is a terminal state — completed tasks cannot change status."
                )));
            }

            return Ok(ToolOutput::error(format!(
                "Cannot transition from '{old_status}' to '{status}'. \
                 Valid transitions from '{old_status}': {}.",
                allowed.join(", ")
            )));
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
                "update_task_status",
                &format!("task:{task_id}"),
                Some(&old_status),
                Some(status),
                note,
                Some(ctx.trace_id),
            )
            .await?;

        let mut response = format!("Task {task_id}: {old_status} → {status}");
        if let Some(n) = note {
            response.push_str(&format!("\nNote: {n}"));
        }
        if metadata_input.is_some() {
            response.push_str("\nMetadata updated.");
        }

        Ok(ToolOutput::success(response))
    }
}

/// Check whether metadata contains any retry-semantic keys (case-insensitive).
///
/// Returns `true` if any top-level key contains the substring "retry" (case-insensitive).
/// This catches `pipeline_retry_count`, `qa_retry_count`, `retry_attempt`, etc.
fn has_retry_semantic_keys(meta: &Value) -> bool {
    if let Some(obj) = meta.as_object() {
        obj.keys().any(|k| k.to_ascii_lowercase().contains("retry"))
    } else {
        false
    }
}

/// Shallow-merge `new_meta` into the task's existing metadata and persist.
async fn merge_and_persist_metadata(
    task_id: &str,
    new_meta: &Value,
    ctx: &ToolContext<'_>,
) -> Result<()> {
    let merged = if let Ok(Some(task)) = ctx.db.get_task_unscoped(task_id).await {
        if let Some(existing_str) = &task.metadata {
            if let Ok(mut existing) = serde_json::from_str::<Value>(existing_str) {
                crate::task_metadata::merge_metadata(&mut existing, new_meta);
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
    ctx.db.update_task_metadata(task_id, &merged_str).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::task_engine::types::{action_type, trigger_type};
    use crate::test_utils::test_helpers::TestHarness;

    async fn create_task(harness: &TestHarness, label: &str) -> String {
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
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_update_status_basic() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Test task").await;
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
        let id = create_task(&harness, "Noted item").await;
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
        let id = create_task(&harness, "Same status").await;
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
    async fn test_update_status_invalid_uuid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": "not-a-uuid", "status": "completed"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid_uuid"));
    }

    #[tokio::test]
    async fn test_update_status_invalid_status() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        assert!(result.content.contains("task_not_found"));
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
                r#type: None,
                dispatch_class: None,
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
        assert!(
            result.content.contains("not a manual task"),
            "expected non-manual rejection, got: {}",
            result.content
        );
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
        let id = create_task(&harness, "Meta item").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        let id = create_task(&harness, "Merge meta").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        let id = create_task(&harness, "489 repro").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        let id = create_task(&harness, "top-level merge").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        let id = create_task(&harness, "Bad meta").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

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
        let tool = UpdateTaskStatusTool;

        // pending → in_progress
        let id = create_task(&harness, "Forward 1").await;
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
        let id2 = create_task(&harness, "Forward 2").await;
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
        let id3 = create_task(&harness, "Forward 3").await;
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
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Unblock item").await;

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

    /// mika#856: cancelled → in_progress reuses the existing task row.
    #[tokio::test]
    async fn test_cancelled_to_in_progress_reuses_row() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Cancel and retry").await;

        // pending → in_progress → cancelled
        tool.execute(
            serde_json::json!({"task_id": id, "status": "in_progress"}),
            &ctx,
        )
        .await
        .unwrap();
        tool.execute(
            serde_json::json!({"task_id": id, "status": "cancelled", "note": "Wrong approach"}),
            &ctx,
        )
        .await
        .unwrap();

        // cancelled → in_progress (revert)
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "in_progress", "note": "Retry with new approach"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "cancelled→in_progress failed: {}",
            result.content
        );
        assert!(result.content.contains("cancelled → in_progress"));

        // Same task_id, status is now in_progress
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");
    }

    /// mika#856: cancelled can only go to in_progress, not blocked/pending/completed.
    #[tokio::test]
    async fn test_cancelled_disallowed_transitions() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        for target in &["pending", "blocked", "completed"] {
            let id = create_task(&harness, &format!("Cancel→{target}")).await;
            tool.execute(
                serde_json::json!({"task_id": id, "status": "cancelled"}),
                &ctx,
            )
            .await
            .unwrap();

            let result = tool
                .execute(serde_json::json!({"task_id": id, "status": target}), &ctx)
                .await
                .unwrap();
            assert!(
                result.is_error,
                "cancelled→{target} should be rejected, got: {}",
                result.content
            );
            assert!(
                result.content.contains("Cannot transition"),
                "missing error preamble for cancelled→{target}: {}",
                result.content
            );
            assert!(
                result.content.contains("in_progress"),
                "should list in_progress as only allowed target for cancelled→{target}: {}",
                result.content
            );
        }
    }

    /// mika#856: metadata-only write on a cancelled task (same-status path) still works.
    #[tokio::test]
    async fn test_cancelled_metadata_only_write() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Cancelled meta write").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "cancelled"}),
            &ctx,
        )
        .await
        .unwrap();

        // Same-status metadata write (status="cancelled" on a cancelled task)
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "cancelled",
                    "metadata": {"cancelled_reason": "duplicate"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "same-status metadata write should succeed: {}",
            result.content
        );
        assert!(result.content.contains("already 'cancelled'"));
        assert!(result.content.contains("Metadata updated"));

        // Status unchanged
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(task.status, "cancelled");
    }

    #[tokio::test]
    async fn test_rejected_backward_transition() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "No regress").await;

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
        let tool = UpdateTaskStatusTool;

        // Test completed → anything
        let id = create_task(&harness, "Completed item").await;
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

        // Test cancelled → disallowed targets (pending, blocked, completed)
        // Note: cancelled → in_progress is allowed (mika#856), tested separately
        let id2 = create_task(&harness, "Cancelled item").await;
        tool.execute(
            serde_json::json!({"task_id": id2, "status": "cancelled"}),
            &ctx,
        )
        .await
        .unwrap();

        for target in &["pending", "blocked", "completed"] {
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
                result
                    .content
                    .contains("Valid transitions from 'cancelled'"),
                "expected valid-transitions message for cancelled→{target}, got: {}",
                result.content
            );
        }
    }

    // -- Terminal-state metadata fallback tests (#617) --

    #[tokio::test]
    async fn test_terminal_metadata_fallback_completed_with_metadata() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Completed with late metadata").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "completed"}),
            &ctx,
        )
        .await
        .unwrap();

        // Try to write metadata with a stale status — should succeed via fallback
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"cost_usd": 14.06, "session_id": "abc123"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            !result.is_error,
            "should succeed with metadata fallback, got: {}",
            result.content
        );
        assert!(
            result.content.contains("terminal"),
            "should mention terminal, got: {}",
            result.content
        );
        assert!(
            result.content.contains("Metadata updated"),
            "should confirm metadata, got: {}",
            result.content
        );

        // Verify status is still completed
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(task.status, "completed");

        // Verify metadata was persisted
        let meta: serde_json::Value =
            serde_json::from_str(task.metadata.as_deref().unwrap()).unwrap();
        assert_eq!(meta["cost_usd"], 14.06);
        assert_eq!(meta["session_id"], "abc123");
    }

    #[tokio::test]
    async fn test_cancelled_to_in_progress_with_metadata() {
        // After mika#856, cancelled → in_progress is a real transition (not a
        // terminal-metadata fallback). The task row is reused.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Cancelled then reverted").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "cancelled"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"reason": "cancel-and-retry"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            !result.is_error,
            "cancelled→in_progress should succeed, got: {}",
            result.content
        );
        assert!(result.content.contains("cancelled → in_progress"));
        assert!(result.content.contains("Metadata updated"));

        // Status actually changed (not the terminal-metadata fallback)
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");

        let meta: serde_json::Value =
            serde_json::from_str(task.metadata.as_deref().unwrap()).unwrap();
        assert_eq!(meta["reason"], "cancel-and-retry");
    }

    #[tokio::test]
    async fn test_terminal_metadata_fallback_merges_with_existing() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Merge test").await;

        // Set initial metadata and complete
        tool.execute(
            serde_json::json!({
                "task_id": id,
                "status": "completed",
                "metadata": {"merge_commit": "abc123", "pr_number": 612}
            }),
            &ctx,
        )
        .await
        .unwrap();

        // Late-arriving callback metadata should merge with existing
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"cost_usd": 14.06, "duration_ms": 839068}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);

        // Verify both old and new metadata are present
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(task.metadata.as_deref().unwrap()).unwrap();
        assert_eq!(meta["merge_commit"], "abc123");
        assert_eq!(meta["pr_number"], 612);
        assert_eq!(meta["cost_usd"], 14.06);
        assert_eq!(meta["duration_ms"], 839068);
    }

    #[tokio::test]
    async fn test_terminal_no_metadata_still_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "No metadata terminal").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "completed"}),
            &ctx,
        )
        .await
        .unwrap();

        // Status-only call without metadata — should still be rejected
        let result = tool
            .execute(
                serde_json::json!({"task_id": id, "status": "in_progress"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            result.is_error,
            "status-only call on terminal task should still be rejected"
        );
        assert!(result.content.contains("terminal state"));
    }

    #[tokio::test]
    async fn test_non_terminal_invalid_transition_with_metadata_still_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let id = create_task(&harness, "Non-terminal reject").await;
        tool.execute(
            serde_json::json!({"task_id": id, "status": "in_progress"}),
            &ctx,
        )
        .await
        .unwrap();

        // in_progress → pending is invalid even with metadata
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "pending",
                    "metadata": {"should": "not be written"}
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(
            result.is_error,
            "non-terminal invalid transition should reject even with metadata"
        );
        assert!(result.content.contains("Valid transitions from"));

        // Verify metadata was NOT written
        let task = ctx.db.get_task(&id).await.unwrap().unwrap();
        assert!(task.metadata.is_none());
    }

    // -- Phantom retry guard tests (#579) --

    /// Helper: create a callback child task for a task.
    async fn create_callback_child(harness: &TestHarness, parent_id: &str, status: &str) -> String {
        let child_id = harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: Some(parent_id.to_string()),
                depth: 1,
                label: "run_claude_pilot".to_string(),
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
                created_by_session: Some("test-session".to_string()),
                created_trace_id: None,
                reference_url: None,
                source: None,
                metadata: None,
                r#type: None,
                dispatch_class: None,
            })
            .await
            .unwrap();

        if status != "pending" {
            harness
                .db
                .update_task_status(&child_id, status)
                .await
                .unwrap();
        }
        child_id
    }

    #[tokio::test]
    async fn test_retry_metadata_succeeds_without_active_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "No callback item").await;
        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"pipeline_retry_count": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "should succeed: {}", result.content);
        assert!(result.content.contains("pending → in_progress"));
    }

    #[tokio::test]
    async fn test_non_retry_metadata_allowed_during_active_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Active dispatch item").await;

        // Transition to in_progress so we can add a callback child
        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "pending").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        // Non-retry metadata should succeed even with active callback
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
        assert!(
            !result.is_error,
            "non-retry metadata should succeed: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_retry_metadata_rejected_with_pending_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Pending callback item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "pending").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"pipeline_retry_count": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error, "should reject: {}", result.content);
        assert!(
            result
                .content
                .contains("retry_metadata_rejected_active_dispatch"),
            "expected structured error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_retry_metadata_rejected_with_in_progress_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "In-progress callback item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "in_progress").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"pipeline_retry_count": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error, "should reject: {}", result.content);
        assert!(
            result
                .content
                .contains("retry_metadata_rejected_active_dispatch")
        );
    }

    #[tokio::test]
    async fn test_retry_variant_key_rejected_with_active_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Variant key item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "pending").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        // "retry_attempt" also matches the retry-semantic check
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"retry_attempt": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.is_error,
            "variant key should be rejected: {}",
            result.content
        );
        assert!(
            result
                .content
                .contains("retry_metadata_rejected_active_dispatch")
        );
    }

    #[tokio::test]
    async fn test_status_only_update_allowed_during_active_callback() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Status only item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "pending").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        // Status-only update (no metadata) should succeed
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "blocked",
                    "note": "Waiting for callback"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "status-only update should succeed: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_retry_metadata_allowed_after_callback_completed() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Completed callback item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "completed").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        // Callback is completed, so retry metadata should be allowed
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"pipeline_retry_count": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "should allow after completed callback: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_retry_metadata_rejected_on_same_status_path() {
        let harness = TestHarness::new();
        let id = create_task(&harness, "Same status retry item").await;

        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();
        create_callback_child(&harness, &id, "pending").await;

        let ctx = harness.ctx();
        let tool = UpdateTaskStatusTool;

        // Same-status path with retry metadata should still be caught
        let result = tool
            .execute(
                serde_json::json!({
                    "task_id": id,
                    "status": "in_progress",
                    "metadata": {"pipeline_retry_count": 1}
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.is_error,
            "same-status path should also enforce guard: {}",
            result.content
        );
        assert!(
            result
                .content
                .contains("retry_metadata_rejected_active_dispatch")
        );
    }

    #[test]
    fn test_has_retry_semantic_keys() {
        assert!(has_retry_semantic_keys(
            &serde_json::json!({"pipeline_retry_count": 1})
        ));
        assert!(has_retry_semantic_keys(
            &serde_json::json!({"qa_retry_count": 0})
        ));
        assert!(has_retry_semantic_keys(
            &serde_json::json!({"retry_attempt": 1})
        ));
        assert!(has_retry_semantic_keys(
            &serde_json::json!({"RETRY_COUNT": 3})
        ));
        assert!(!has_retry_semantic_keys(
            &serde_json::json!({"claude_pilot": {"branch": "main"}})
        ));
        assert!(!has_retry_semantic_keys(
            &serde_json::json!({"ci_fix_count": 2})
        ));
        assert!(!has_retry_semantic_keys(&serde_json::json!({})));
        assert!(!has_retry_semantic_keys(&serde_json::json!(
            "not an object"
        )));
        // Nested retry keys are intentionally excluded — only top-level keys trigger the guard
        assert!(!has_retry_semantic_keys(
            &serde_json::json!({"dispatch_info": {"retry_count": 1}})
        ));
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
        assert!(is_valid_transition("cancelled", "in_progress")); // mika#856
        assert!(!is_valid_transition("cancelled", "pending"));
        assert!(!is_valid_transition("cancelled", "blocked"));
        assert!(!is_valid_transition("cancelled", "completed"));
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
        assert_eq!(allowed_transitions("cancelled"), &["in_progress"]); // mika#856
        assert!(allowed_transitions("unknown").is_empty());
    }
}
