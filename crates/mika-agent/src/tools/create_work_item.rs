use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::NewTask;
use crate::task_engine::types::{action_type, trigger_type};

/// Maximum agent-created work items per session (Guard 5).
const MAX_WORK_ITEMS_PER_SESSION: i64 = 5;

const VALID_SOURCES: &[&str] = &["user_request", "github_issue", "team_run", "self_dev"];

pub struct CreateWorkItemTool;

#[async_trait]
impl Tool for CreateWorkItemTool {
    fn name(&self) -> &str {
        "create_work_item"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_work_item".to_string(),
            description: "Create a trackable work item to represent a piece of work. \
                Work items can be linked to external references (GitHub issues, URLs) \
                and progressed through status stages. Use for significant tasks like \
                feature implementation, research projects, or items waiting on external input. \
                Cannot be used during callback turns. Max 5 agent-created items per session. \
                Max nesting depth of 3."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Description of the work item"
                    },
                    "reference_url": {
                        "type": "string",
                        "description": "Optional URL reference (GitHub issue, document, etc.)"
                    },
                    "source": {
                        "type": "string",
                        "enum": ["user_request", "github_issue", "team_run", "self_dev"],
                        "description": "Origin of the work item"
                    },
                    "parent_task_id": {
                        "type": "string",
                        "description": "Optional parent work item ID for subtask nesting"
                    }
                },
                "required": ["label"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let label = input["label"].as_str().unwrap_or("").trim();
        let reference_url = input["reference_url"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let source = input["source"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let parent_task_id = input["parent_task_id"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        // Validate inputs
        if label.is_empty() {
            return Ok(ToolOutput::error("'label' is required."));
        }
        if label.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'label' too long: {} characters (max: {MAX_INPUT_LEN})",
                label.len()
            )));
        }
        if let Some(url) = reference_url
            && url.len() > MAX_INPUT_LEN
        {
            return Ok(ToolOutput::error("'reference_url' is too long."));
        }
        if let Some(src) = source
            && !VALID_SOURCES.contains(&src)
        {
            return Ok(ToolOutput::error(format!(
                "Invalid source '{}'. Must be one of: {}",
                src,
                VALID_SOURCES.join(", ")
            )));
        }

        // Guard 3: Callback turns block ALL work item creation
        if ctx.is_callback_turn {
            return Ok(ToolOutput::error(
                "Cannot create work items during a callback turn. Answer the question and return.",
            ));
        }

        // Guard 1: No top-level creation from task context
        if ctx.is_task_context && parent_task_id.is_none() {
            return Ok(ToolOutput::error(
                "Cannot create top-level work items from within a task context. \
                 Provide a parent_task_id to create a subtask instead.",
            ));
        }

        // Guard 2: Depth cap (application-level check before hitting DB constraint)
        let depth = if let Some(parent_id) = parent_task_id {
            match ctx.db.get_task_depth(parent_id).await? {
                Some(parent_depth) => {
                    let child_depth = parent_depth + 1;
                    if child_depth > 3 {
                        return Ok(ToolOutput::error(
                            "Maximum nesting depth (3) exceeded. Cannot create deeper subtasks.",
                        ));
                    }
                    child_depth
                }
                None => {
                    return Ok(ToolOutput::error(format!(
                        "Parent task '{parent_id}' not found."
                    )));
                }
            }
        } else {
            0
        };

        // Guard 5: Cap agent-created work items per session
        if source != Some("user_request") {
            let count = ctx.db.count_session_work_items(ctx.session_id).await?;
            if count >= MAX_WORK_ITEMS_PER_SESSION {
                return Ok(ToolOutput::error(format!(
                    "Maximum of {MAX_WORK_ITEMS_PER_SESSION} agent-created work items per session reached."
                )));
            }
        }

        let task = NewTask {
            agent_id: ctx.db.agent_id.clone(),
            team_run_id: None,
            parent_task_id: parent_task_id.map(|s| s.to_string()),
            depth,
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
            created_by_session: Some(ctx.session_id.to_string()),
            created_trace_id: Some(ctx.trace_id.to_string()),
            reference_url: reference_url.map(|s| s.to_string()),
            source: source.map(|s| s.to_string()),
            metadata: None,
        };

        let id = ctx.db.create_task(task).await?;

        // Log audit event
        let after_value = if let Some(url) = reference_url {
            format!("created — {label} (ref: {url})")
        } else {
            format!("created — {label}")
        };
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "create_work_item",
                &format!("task:{id}"),
                None,
                Some(&*after_value),
                None,
                Some(ctx.trace_id),
            )
            .await?;

        let mut response = format!("Work item created: {id}\nLabel: {label}\nStatus: pending");
        if let Some(url) = reference_url {
            response.push_str(&format!("\nReference: {url}"));
        }
        if let Some(src) = source {
            response.push_str(&format!("\nSource: {src}"));
        }
        if let Some(pid) = parent_task_id {
            response.push_str(&format!("\nParent: {pid}"));
        }

        Ok(ToolOutput::success(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_create_work_item_basic() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Implement feature X",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Work item created"));
        assert!(result.content.contains("Implement feature X"));
    }

    #[tokio::test]
    async fn test_create_work_item_with_reference() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Fix issue #42",
                    "reference_url": "https://github.com/org/repo/issues/42",
                    "source": "github_issue"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result
                .content
                .contains("Reference: https://github.com/org/repo/issues/42")
        );
        assert!(result.content.contains("Source: github_issue"));
    }

    #[tokio::test]
    async fn test_create_work_item_empty_label() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'label' is required"));
    }

    #[tokio::test]
    async fn test_create_work_item_callback_guard() {
        let harness = TestHarness::new();
        let mut ctx = harness.ctx();
        ctx.is_callback_turn = true;
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({"label": "Should fail", "source": "user_request"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("callback turn"));
    }

    #[tokio::test]
    async fn test_create_work_item_task_context_guard() {
        let harness = TestHarness::new();
        let mut ctx = harness.ctx();
        ctx.is_task_context = true;
        let tool = CreateWorkItemTool;

        // Top-level creation blocked
        let result = tool
            .execute(
                serde_json::json!({"label": "Should fail", "source": "user_request"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("top-level"));
    }

    #[tokio::test]
    async fn test_create_work_item_session_cap() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create MAX_WORK_ITEMS_PER_SESSION items
        let mut item_ids = Vec::new();
        for i in 0..MAX_WORK_ITEMS_PER_SESSION {
            let result = tool
                .execute(serde_json::json!({"label": format!("Item {i}")}), &ctx)
                .await
                .unwrap();
            assert!(!result.is_error, "item {i} failed: {}", result.content);
            let id = result
                .content
                .lines()
                .next()
                .unwrap()
                .strip_prefix("Work item created: ")
                .unwrap()
                .to_string();
            item_ids.push(id);
        }

        // 6th should be rejected (all 5 are active)
        let result = tool
            .execute(serde_json::json!({"label": "One too many"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Maximum"));

        // Complete one item — should free up a slot
        ctx.db
            .update_task_status(&item_ids[0], "completed")
            .await
            .unwrap();

        // Now creating a 6th should succeed (only 4 active)
        let result = tool
            .execute(serde_json::json!({"label": "After completing one"}), &ctx)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "should succeed after completing one: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_session_cap_excludes_user_request() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create MAX items with source != user_request
        for i in 0..MAX_WORK_ITEMS_PER_SESSION {
            tool.execute(
                serde_json::json!({"label": format!("Agent item {i}")}),
                &ctx,
            )
            .await
            .unwrap();
        }

        // user_request should still work
        let result = tool
            .execute(
                serde_json::json!({"label": "User request", "source": "user_request"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "user_request should bypass cap: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_with_parent() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create parent
        let parent_result = tool
            .execute(
                serde_json::json!({"label": "Parent work item", "source": "user_request"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!parent_result.is_error);
        // Extract the UUID from the response
        let parent_id = parent_result
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap();

        // Create child
        let child_result = tool
            .execute(
                serde_json::json!({
                    "label": "Child work item",
                    "parent_task_id": parent_id,
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !child_result.is_error,
            "got error: {}",
            child_result.content
        );
        assert!(
            child_result
                .content
                .contains(&format!("Parent: {parent_id}"))
        );
    }

    #[tokio::test]
    async fn test_create_work_item_depth_cap() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create a chain: depth 0 → 1 → 2 → 3 → 4 (should fail at 4)
        let mut parent_id = None;
        for depth in 0..=3 {
            let mut input = serde_json::json!({
                "label": format!("Level {depth}"),
                "source": "user_request"
            });
            if let Some(pid) = &parent_id {
                input["parent_task_id"] = serde_json::json!(pid);
            }
            let result = tool.execute(input, &ctx).await.unwrap();
            if depth <= 3 {
                assert!(!result.is_error, "depth {depth} failed: {}", result.content);
                parent_id = Some(
                    result
                        .content
                        .lines()
                        .next()
                        .unwrap()
                        .strip_prefix("Work item created: ")
                        .unwrap()
                        .to_string(),
                );
            }
        }

        // Depth 4 should fail
        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Too deep",
                    "parent_task_id": parent_id.unwrap(),
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("depth"));
    }

    #[tokio::test]
    async fn test_create_work_item_invalid_parent() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Orphan",
                    "parent_task_id": "nonexistent-id",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }
}
