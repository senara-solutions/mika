use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::{NewTask, TASK_TYPE_ISSUE, Task, VALID_TASK_TYPES, is_unique_violation};
use crate::task_engine::types::{action_type, trigger_type};

/// Format a success response for a deduplicated work item (already exists).
fn format_dedup_response(existing: &Task) -> String {
    let mut response = format!(
        "Work item already exists for this reference: {}\n\
         Label: {}\n\
         Status: {}",
        existing.id, existing.label, existing.status
    );
    if existing.r#type != TASK_TYPE_ISSUE {
        response.push_str(&format!("\nType: {}", existing.r#type));
    }
    if let Some(url) = &existing.reference_url {
        response.push_str(&format!("\nReference: {url}"));
    }
    response.push_str(&format!(
        "\nCreated: {}\n\nReuse this work item ID for subsequent operations.",
        existing.created_at
    ));
    response
}

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
                Max nesting depth of 3. Set 'type' to 'milestone' or 'project' to mark a parent \
                container that aggregates child 'issue' work items via parent_task_id."
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
                    },
                    "type": {
                        "type": "string",
                        "enum": ["issue", "milestone", "project"],
                        "description": "Work item kind. Defaults to 'issue'. Use 'milestone' or 'project' \
                                        for parent containers that group child issues via parent_task_id."
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
        // Empty/whitespace-only `type` falls back to the default ("issue").
        let task_type_input = input["type"]
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
        if let Some(t) = task_type_input
            && !VALID_TASK_TYPES.contains(&t)
        {
            return Ok(ToolOutput::error(format!(
                "Invalid type '{}'. Must be one of: {}",
                t,
                VALID_TASK_TYPES.join(", ")
            )));
        }
        // Resolved type used both for the row and for the response message.
        let task_type = task_type_input.unwrap_or(TASK_TYPE_ISSUE);

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

        // Validate parent_task_id format before DB lookup
        if let Some(parent_id) = parent_task_id
            && let Err(e) = super::validate_uuid("parent_task_id", parent_id)
        {
            return Ok(e);
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

        // Dedup: check for existing active work items before session cap and INSERT.
        // Dedup returns existing items as success — it's not a "new creation" so it
        // must run before the session cap guard to avoid blocking legitimate reuse.

        // Dedup path A (reference_url provided): pre-check by URL
        if let Some(url) = reference_url
            && let Some(existing) = ctx.db.find_active_work_item_by_ref_url(url).await?
        {
            ctx.db
                .log_audit_event(
                    ctx.session_id,
                    "create_work_item",
                    &format!("task:{}", existing.id),
                    None,
                    Some(&format!("dedup_reused — {} (ref: {url})", existing.label)),
                    None,
                    Some(ctx.trace_id),
                )
                .await?;
            return Ok(ToolOutput::success(format_dedup_response(&existing)));
        }

        // Dedup path B (label-only, no reference_url): pre-check by label
        if reference_url.is_none()
            && let Some(existing) = ctx.db.find_active_work_item_by_label(label).await?
        {
            ctx.db
                .log_audit_event(
                    ctx.session_id,
                    "create_work_item",
                    &format!("task:{}", existing.id),
                    None,
                    Some(&format!("dedup_reused — {}", existing.label)),
                    None,
                    Some(ctx.trace_id),
                )
                .await?;
            return Ok(ToolOutput::success(format_dedup_response(&existing)));
        }

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
            r#type: Some(task_type.to_string()),
        };

        // Dedup path A (reference_url provided): attempt INSERT, catch UNIQUE violation
        match ctx.db.create_task(task).await {
            Ok(id) => {
                // Log audit event for new creation
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

                let mut response =
                    format!("Work item created: {id}\nLabel: {label}\nStatus: pending");
                if let Some(url) = reference_url {
                    response.push_str(&format!("\nReference: {url}"));
                }
                if let Some(src) = source {
                    response.push_str(&format!("\nSource: {src}"));
                }
                if let Some(pid) = parent_task_id {
                    response.push_str(&format!("\nParent: {pid}"));
                }
                // Surface non-default types so the caller can confirm. Hide for "issue"
                // to keep the common-case response compact.
                if task_type != TASK_TYPE_ISSUE {
                    response.push_str(&format!("\nType: {task_type}"));
                }
                Ok(ToolOutput::success(response))
            }
            Err(err) if reference_url.is_some() && is_unique_violation(&err) => {
                // UNIQUE constraint on (agent_id, reference_url) fired — find the existing item
                let url = reference_url.unwrap();
                if let Some(existing) = ctx.db.find_active_work_item_by_ref_url(url).await? {
                    // Log dedup reuse in audit
                    ctx.db
                        .log_audit_event(
                            ctx.session_id,
                            "create_work_item",
                            &format!("task:{}", existing.id),
                            None,
                            Some(&format!("dedup_reused — {} (ref: {url})", existing.label)),
                            None,
                            Some(ctx.trace_id),
                        )
                        .await?;
                    Ok(ToolOutput::success(format_dedup_response(&existing)))
                } else {
                    // Edge case: item transitioned to terminal between INSERT and SELECT.
                    // Retry the INSERT once.
                    let retry_task = NewTask {
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
                        reference_url: Some(url.to_string()),
                        source: source.map(|s| s.to_string()),
                        metadata: None,
                        r#type: Some(task_type.to_string()),
                    };
                    let id = ctx.db.create_task(retry_task).await?;
                    let after_value =
                        format!("created — {label} (ref: {url}, retry after dedup race)");
                    ctx.db
                        .log_audit_event(
                            ctx.session_id,
                            "create_work_item",
                            &format!("task:{id}"),
                            None,
                            Some(&after_value),
                            None,
                            Some(ctx.trace_id),
                        )
                        .await?;
                    let mut response =
                        format!("Work item created: {id}\nLabel: {label}\nStatus: pending");
                    response.push_str(&format!("\nReference: {url}"));
                    if let Some(src) = source {
                        response.push_str(&format!("\nSource: {src}"));
                    }
                    if let Some(pid) = parent_task_id {
                        response.push_str(&format!("\nParent: {pid}"));
                    }
                    if task_type != TASK_TYPE_ISSUE {
                        response.push_str(&format!("\nType: {task_type}"));
                    }
                    Ok(ToolOutput::success(response))
                }
            }
            Err(err) => Err(err),
        }
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
    async fn test_create_work_item_invalid_parent_uuid() {
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
        assert!(
            result.content.contains("invalid_uuid"),
            "expected UUID error, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_parent_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "Orphan",
                    "parent_task_id": "00000000-0000-0000-0000-000000000000",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    // ===== Dedup tests (issue #303) =====

    #[tokio::test]
    async fn test_create_work_item_dedup_by_reference_url() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let url = "https://github.com/org/repo/issues/303";

        // First creation succeeds normally
        let result1 = tool
            .execute(
                serde_json::json!({
                    "label": "Fix issue #303",
                    "reference_url": url,
                    "source": "github_issue"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result1.is_error,
            "first create failed: {}",
            result1.content
        );
        assert!(result1.content.contains("Work item created"));
        let id1 = result1
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap()
            .to_string();

        // Second creation with same URL returns existing item
        let result2 = tool
            .execute(
                serde_json::json!({
                    "label": "Fix issue #303 (retry)",
                    "reference_url": url,
                    "source": "github_issue"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result2.is_error, "dedup failed: {}", result2.content);
        assert!(
            result2.content.contains("already exists"),
            "expected dedup response, got: {}",
            result2.content
        );
        assert!(
            result2.content.contains(&id1),
            "should return original ID {id1}, got: {}",
            result2.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_dedup_by_label() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // First creation (no reference_url)
        let result1 = tool
            .execute(
                serde_json::json!({
                    "label": "Research memory patterns",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result1.is_error);
        let id1 = result1
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap()
            .to_string();

        // Second creation with same label returns existing
        let result2 = tool
            .execute(
                serde_json::json!({
                    "label": "Research memory patterns",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result2.is_error);
        assert!(result2.content.contains("already exists"));
        assert!(result2.content.contains(&id1));
    }

    #[tokio::test]
    async fn test_create_work_item_dedup_case_insensitive_label() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create with one casing
        let result1 = tool
            .execute(
                serde_json::json!({
                    "label": "Fix Login Bug",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result1.is_error);
        let id1 = result1
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap()
            .to_string();

        // Same label different casing → dedup
        let result2 = tool
            .execute(
                serde_json::json!({
                    "label": "fix login bug",
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result2.is_error);
        assert!(result2.content.contains("already exists"));
        assert!(result2.content.contains(&id1));
    }

    #[tokio::test]
    async fn test_create_work_item_allows_after_terminal() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let url = "https://github.com/org/repo/issues/99";

        // Create and complete item
        let result1 = tool
            .execute(
                serde_json::json!({
                    "label": "Old task",
                    "reference_url": url,
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result1.is_error);
        let id1 = result1
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap()
            .to_string();
        ctx.db.update_task_status(&id1, "completed").await.unwrap();

        // New creation with same URL should succeed (old is terminal)
        let result2 = tool
            .execute(
                serde_json::json!({
                    "label": "Retry task",
                    "reference_url": url,
                    "source": "user_request"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result2.is_error, "failed: {}", result2.content);
        assert!(
            result2.content.contains("Work item created"),
            "expected new creation, got: {}",
            result2.content
        );
        // Should be a different ID
        let id2 = result2
            .content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .unwrap();
        assert_ne!(id1, id2, "should be a new item, not the old completed one");
    }

    #[tokio::test]
    async fn test_create_work_item_dedup_cross_agent() {
        // Different agents should be able to create items with the same reference_url
        let harness1 = TestHarness::new();
        let harness2 = TestHarness::with_agent("other-agent");
        let ctx1 = harness1.ctx();
        let ctx2 = harness2.ctx();
        let tool = CreateWorkItemTool;

        let url = "https://github.com/org/repo/issues/50";

        let result1 = tool
            .execute(
                serde_json::json!({
                    "label": "Agent 1 task",
                    "reference_url": url,
                    "source": "user_request"
                }),
                &ctx1,
            )
            .await
            .unwrap();
        assert!(!result1.is_error);

        let result2 = tool
            .execute(
                serde_json::json!({
                    "label": "Agent 2 task",
                    "reference_url": url,
                    "source": "user_request"
                }),
                &ctx2,
            )
            .await
            .unwrap();
        assert!(!result2.is_error, "cross-agent failed: {}", result2.content);
        assert!(
            result2.content.contains("Work item created"),
            "different agent should create new item, got: {}",
            result2.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_dedup_skips_session_counter() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        // Create 4 unique items (leaves 1 slot)
        for i in 0..4 {
            tool.execute(
                serde_json::json!({"label": format!("Unique item {i}")}),
                &ctx,
            )
            .await
            .unwrap();
        }

        // Create item with URL
        tool.execute(
            serde_json::json!({
                "label": "URL item",
                "reference_url": "https://example.com/issue/1"
            }),
            &ctx,
        )
        .await
        .unwrap();
        // Now at 5/5 cap

        // Dedup hit on URL item should NOT consume a slot
        let dedup_result = tool
            .execute(
                serde_json::json!({
                    "label": "URL item retry",
                    "reference_url": "https://example.com/issue/1"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!dedup_result.is_error);
        assert!(dedup_result.content.contains("already exists"));

        // The cap is still at 5/5 — dedup didn't consume a slot.
        // Verify by trying to create a truly new item (should fail, cap reached)
        let over_cap = tool
            .execute(serde_json::json!({"label": "Truly new item"}), &ctx)
            .await
            .unwrap();
        assert!(
            over_cap.is_error,
            "should be at cap, but got: {}",
            over_cap.content
        );
        assert!(over_cap.content.contains("Maximum"));
    }

    // ===== `type` column tests (issue #595) =====

    /// Helper: extract the new work-item ID from a successful create response.
    fn extract_id(content: &str) -> &str {
        content
            .lines()
            .next()
            .unwrap()
            .strip_prefix("Work item created: ")
            .expect("response should start with 'Work item created: '")
    }

    #[tokio::test]
    async fn test_create_work_item_type_defaults_to_issue() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({"label": "no type", "source": "user_request"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        // Default type should NOT appear in the response message — it's the common case.
        assert!(
            !result.content.contains("Type:"),
            "default type should be hidden in response: {}",
            result.content
        );

        // Row should persist with type='issue'.
        let id = extract_id(&result.content);
        let task = ctx
            .db
            .get_manual_task(id)
            .await
            .unwrap()
            .expect("just-created work item should be retrievable");
        assert_eq!(task.r#type, "issue");
    }

    #[tokio::test]
    async fn test_create_work_item_type_milestone() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "milestone parent",
                    "source": "user_request",
                    "type": "milestone"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result.content.contains("Type: milestone"),
            "non-default type should appear in response: {}",
            result.content
        );

        let id = extract_id(&result.content);
        let task = ctx.db.get_manual_task(id).await.unwrap().unwrap();
        assert_eq!(task.r#type, "milestone");
    }

    #[tokio::test]
    async fn test_create_work_item_type_project() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "project parent",
                    "source": "user_request",
                    "type": "project"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Type: project"));

        let id = extract_id(&result.content);
        let task = ctx.db.get_manual_task(id).await.unwrap().unwrap();
        assert_eq!(task.r#type, "project");
    }

    #[tokio::test]
    async fn test_create_work_item_type_explicit_issue_hidden() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "explicit issue",
                    "source": "user_request",
                    "type": "issue"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        // Explicit "issue" should be treated identically to omitted: hidden.
        assert!(
            !result.content.contains("Type:"),
            "explicit issue type should be hidden: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_create_work_item_type_invalid() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "bad type",
                    "source": "user_request",
                    "type": "epic"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error, "should reject invalid type");
        assert!(
            result.content.contains("Invalid type 'epic'"),
            "error should name the offending value: {}",
            result.content
        );
        assert!(
            result.content.contains("issue, milestone, project"),
            "error should list valid types: {}",
            result.content
        );

        // Verify no row was created.
        let count = ctx
            .db
            .count_session_work_items(ctx.session_id)
            .await
            .unwrap();
        assert_eq!(count, 0, "invalid type should not insert a row");
    }

    #[tokio::test]
    async fn test_create_work_item_type_empty_string_defaults() {
        // Whitespace-only `type` should fall back to the default rather than error.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "label": "whitespace type",
                    "source": "user_request",
                    "type": "   "
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        let id = extract_id(&result.content);
        let task = ctx.db.get_manual_task(id).await.unwrap().unwrap();
        assert_eq!(task.r#type, "issue");
    }

    #[tokio::test]
    async fn test_create_work_item_milestone_with_parent() {
        // Existing depth/parent guards should still apply with non-default types.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CreateWorkItemTool;

        let parent = tool
            .execute(
                serde_json::json!({
                    "label": "milestone container",
                    "source": "user_request",
                    "type": "milestone"
                }),
                &ctx,
            )
            .await
            .unwrap();
        let parent_id = extract_id(&parent.content).to_string();

        let child = tool
            .execute(
                serde_json::json!({
                    "label": "child issue",
                    "source": "user_request",
                    "parent_task_id": parent_id,
                    "type": "issue"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!child.is_error, "got error: {}", child.content);
        let child_id = extract_id(&child.content);
        let child_task = ctx.db.get_manual_task(child_id).await.unwrap().unwrap();
        assert_eq!(child_task.r#type, "issue");
        assert_eq!(
            child_task.parent_task_id.as_deref(),
            Some(parent_id.as_str())
        );
    }
}
