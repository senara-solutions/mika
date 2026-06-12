use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::ForcePromoteResult;

/// Known dispatch classes (keep in sync with `DISPATCH_CLASSES` in engine.rs
/// and `derive_dispatch_class` in skills/executor.rs).
const VALID_DISPATCH_CLASSES: &[&str] = &["implement", "groom"];

pub struct PromoteDeferredCallbackTool;

#[async_trait]
impl Tool for PromoteDeferredCallbackTool {
    fn name(&self) -> &str {
        "promote_deferred_callback"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "promote_deferred_callback".to_string(),
            description:
                "Force-promote the next pending deferred dispatch wrapper for a given dispatch class. \
                Fail-closed: if the per-class dispatch slot is already occupied by an active callback, \
                the promotion is refused. In that case, surface the rejection to the operator via \
                send_message — the operator can use `mika tasks promote-deferred <class> --override` \
                to cancel the blocker and force promotion."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "dispatch_class": {
                        "type": "string",
                        "description": "The dispatch class to promote a deferred wrapper for: 'implement' or 'groom'",
                        "enum": ["implement", "groom"]
                    }
                },
                "required": ["dispatch_class"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let dispatch_class = input["dispatch_class"].as_str().unwrap_or("").trim();
        if dispatch_class.is_empty() {
            return Ok(ToolOutput::error("'dispatch_class' is required."));
        }
        if !VALID_DISPATCH_CLASSES.contains(&dispatch_class) {
            return Ok(ToolOutput::error(format!(
                "Invalid dispatch_class '{dispatch_class}'. Must be one of: {}",
                VALID_DISPATCH_CLASSES.join(", ")
            )));
        }

        let result = ctx
            .db
            .force_promote_deferred_for_class(dispatch_class)
            .await?;

        match result {
            ForcePromoteResult::Promoted { task_id } => {
                ctx.db
                    .log_audit_event(
                        ctx.session_id,
                        "deferred_dispatch_force_promote_succeeded",
                        &format!("dispatch_class:{dispatch_class}"),
                        None,
                        Some(&format!("promoted:{task_id}")),
                        None,
                        Some(ctx.trace_id),
                    )
                    .await?;

                Ok(ToolOutput::success(format!(
                    "Deferred wrapper promoted for class '{dispatch_class}'. Promoted task ID: {task_id}"
                )))
            }
            ForcePromoteResult::RejectedSlotBusy { blocking_label } => {
                ctx.db
                    .log_audit_event(
                        ctx.session_id,
                        "deferred_dispatch_force_promote_rejected_slot_busy",
                        &format!("dispatch_class:{dispatch_class}"),
                        None,
                        Some("rejected_slot_busy"),
                        Some(&format!("blocker: {blocking_label}")),
                        Some(ctx.trace_id),
                    )
                    .await?;

                Ok(ToolOutput::error(serde_json::to_string(
                    &serde_json::json!({
                        "error": "slot_busy",
                        "dispatch_class": dispatch_class,
                        "blocking_label": blocking_label,
                        "message": format!(
                            "Cannot promote: dispatch slot for class '{dispatch_class}' is occupied by '{blocking_label}'. \
                            Surface this to the operator via send_message — they can use \
                            `mika tasks promote-deferred {dispatch_class} --override` to cancel the blocker and force promotion."
                        )
                    }),
                )?))
            }
            ForcePromoteResult::NoPendingWrapper => Ok(ToolOutput::error(format!(
                "No pending deferred wrapper for class '{dispatch_class}'."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::test_utils::test_helpers::TestHarness;

    fn deferred_callback_task(agent_id: &str, dispatch_class: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot:deferred".to_string(),
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
            dispatch_class: Some(dispatch_class.to_string()),
        }
    }

    fn real_callback_task(agent_id: &str, dispatch_class: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
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
            dispatch_class: Some(dispatch_class.to_string()),
        }
    }

    #[tokio::test]
    async fn test_promote_happy_path() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;

        // Create a pending deferred wrapper
        harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "Expected success but got error: {}",
            result.content
        );
        assert!(
            result.content.contains("promoted"),
            "Should contain 'promoted': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_slot_busy() {
        let harness = TestHarness::new();
        let agent_id = &harness.db.agent_id;

        // Create a pending deferred wrapper
        harness
            .db
            .create_task(deferred_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        // Create a pending real callback (occupies the slot — pending non-deferred
        // callbacks count as slot-occupying per has_any_active_callback_for_class)
        harness
            .db
            .create_task(real_callback_task(agent_id, "implement"))
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error, "Expected error but got success");
        assert!(
            result.content.contains("slot_busy"),
            "Should contain 'slot_busy': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_no_pending_wrapper() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "implement"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error, "Expected error but got success");
        assert!(
            result.content.contains("No pending deferred wrapper"),
            "Should mention no pending wrapper: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_promote_invalid_class() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool
            .execute(serde_json::json!({"dispatch_class": "invalid"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid dispatch_class"));
    }

    #[tokio::test]
    async fn test_promote_missing_class() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = PromoteDeferredCallbackTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'dispatch_class' is required"));
    }
}
