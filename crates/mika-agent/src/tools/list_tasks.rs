use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::{TASK_TYPE_ISSUE, format_ts};

pub struct ListTasksTool;

/// Status display order: active statuses first, then terminal.
const STATUS_ORDER: &[&str] = &[
    "pending",
    "in_progress",
    "blocked",
    "completed",
    "cancelled",
];

const FILTER_GUIDANCE_NOTE: &str = "All items are returned in this response. \
    Filter by status= only when you need a strict subset for a subsequent action \
    — not to verify counts already shown in summary.";

/// Build a status-count summary line from the result set.
/// Returns e.g. "50 items total — 2 blocked, 48 completed" (omits zero-count statuses).
fn status_summary(items: &[(crate::db::Task, Option<i64>)]) -> String {
    let total = items.len();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (task, _) in items {
        *counts.entry(task.status.as_str()).or_insert(0) += 1;
    }

    let parts: Vec<String> = STATUS_ORDER
        .iter()
        .filter_map(|s| counts.get(s).map(|c| format!("{c} {s}")))
        .collect();

    if parts.is_empty() {
        format!("{total} items total")
    } else {
        format!("{total} items total \u{2014} {}", parts.join(", "))
    }
}

#[async_trait]
impl Tool for ListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "List tracked tasks with optional filtering by status and source. \
                Returns up to 50 tasks, ordered by creation date (newest first). \
                Check this before creating new tasks to avoid duplicates."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["pending", "in_progress", "blocked", "completed", "cancelled"],
                        "description": "Filter by status"
                    },
                    "source": {
                        "type": "string",
                        "enum": ["user_request", "github_issue", "team_run", "self_dev"],
                        "description": "Filter by source"
                    },
                    "include_children": {
                        "type": "boolean",
                        "description": "Include child task count for each task (default: false)"
                    }
                }
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let status = input["status"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let source = input["source"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let include_children = input["include_children"].as_bool().unwrap_or(false);

        const VALID_SOURCES: &[&str] = &["user_request", "github_issue", "team_run", "self_dev"];

        if let Some(s) = status
            && !STATUS_ORDER.contains(&s)
        {
            return Ok(ToolOutput::error(format!(
                "Invalid status '{}'. Must be one of: {}",
                s,
                STATUS_ORDER.join(", ")
            )));
        }
        if let Some(s) = source
            && !VALID_SOURCES.contains(&s)
        {
            return Ok(ToolOutput::error(format!(
                "Invalid source '{}'. Must be one of: {}",
                s,
                VALID_SOURCES.join(", ")
            )));
        }

        let items = ctx
            .db
            .list_manual_tasks(status, source, include_children)
            .await?;

        if items.is_empty() {
            let mut msg = "No tasks found".to_string();
            if let Some(s) = status {
                msg.push_str(&format!(" with status '{s}'"));
            }
            if let Some(s) = source {
                msg.push_str(&format!(" from source '{s}'"));
            }
            msg.push('.');
            return Ok(ToolOutput::success(msg));
        }

        let mut lines = Vec::new();
        lines.push(format!("Tasks ({}):\n", items.len()));

        for (task, child_count) in &items {
            let created = format_ts(&task.created_at);
            let ref_url = task
                .reference_url
                .as_deref()
                .map(|u| format!(" ref:{u}"))
                .unwrap_or_default();
            let src = task
                .source
                .as_deref()
                .map(|s| format!(" src:{s}"))
                .unwrap_or_default();
            // Hide the default ("issue") to keep the common case compact;
            // surface "milestone"/"project" so parent containers stand out.
            let task_type = if task.r#type == TASK_TYPE_ISSUE {
                String::new()
            } else {
                format!(" type:{}", task.r#type)
            };
            let children = child_count
                .map(|c| format!(" children:{c}"))
                .unwrap_or_default();
            let pid = task
                .process_id
                .map(|p| format!(" pid:{p}"))
                .unwrap_or_default();

            lines.push(format!(
                "- [{status}] {id} {label} (created:{created}{ref_url}{src}{task_type}{children}{pid})",
                status = task.status,
                id = task.id,
                label = task.label,
            ));
        }

        // Append status-count summary
        lines.push(String::new()); // blank line separator
        lines.push(format!("Summary: {}", status_summary(&items)));

        // Append filter guidance note for fully unfiltered calls only
        if status.is_none() && source.is_none() {
            lines.push(format!("Note: {FILTER_GUIDANCE_NOTE}"));
        }

        Ok(ToolOutput::success(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::task_engine::types::{action_type, trigger_type};
    use crate::test_utils::test_helpers::TestHarness;

    async fn create_test_task(
        harness: &TestHarness,
        label: &str,
        source: Option<&str>,
        reference_url: Option<&str>,
    ) -> String {
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
                reference_url: reference_url.map(|s| s.to_string()),
                source: source.map(|s| s.to_string()),
                metadata: None,
                r#type: None,
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
        assert!(result.content.contains("No tasks found"));
    }

    #[tokio::test]
    async fn test_list_tasks_basic() {
        let harness = TestHarness::new();
        create_test_task(&harness, "Item A", Some("user_request"), None).await;
        create_test_task(
            &harness,
            "Item B",
            Some("github_issue"),
            Some("https://github.com/org/repo/issues/1"),
        )
        .await;
        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Item A"));
        assert!(result.content.contains("Item B"));
        assert!(result.content.contains("Tasks (2)"));
    }

    #[tokio::test]
    async fn test_list_tasks_filter_by_status() {
        let harness = TestHarness::new();
        let id = create_test_task(&harness, "Active item", Some("user_request"), None).await;
        create_test_task(&harness, "Pending item", Some("user_request"), None).await;

        // Move one to in_progress
        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool
            .execute(serde_json::json!({"status": "in_progress"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Active item"));
        assert!(!result.content.contains("Pending item"));
    }

    #[tokio::test]
    async fn test_list_tasks_filter_by_source() {
        let harness = TestHarness::new();
        create_test_task(&harness, "User item", Some("user_request"), None).await;
        create_test_task(&harness, "GH item", Some("github_issue"), None).await;
        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool
            .execute(serde_json::json!({"source": "github_issue"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("GH item"));
        assert!(!result.content.contains("User item"));
        // Source-only filter should NOT show the guidance note (not fully unfiltered)
        assert!(
            !result.content.contains("Note:"),
            "note should not appear on source-filtered calls: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_tasks_with_children() {
        let harness = TestHarness::new();
        let parent_id = create_test_task(&harness, "Parent item", Some("user_request"), None).await;
        // Create a child
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: Some(parent_id.clone()),
                depth: 1,
                label: "Child item".to_string(),
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
            })
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool
            .execute(serde_json::json!({"include_children": true}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        // Parent should show children:1
        assert!(
            result.content.contains("children:1"),
            "should show child count: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_tasks_summary_mixed_statuses() {
        let harness = TestHarness::new();
        // Create items in different statuses
        create_test_task(&harness, "Item 1", Some("user_request"), None).await;
        create_test_task(&harness, "Item 2", Some("user_request"), None).await;
        let id3 = create_test_task(&harness, "Item 3", Some("user_request"), None).await;
        let id4 = create_test_task(&harness, "Item 4", Some("user_request"), None).await;

        harness
            .db
            .update_manual_task_status(&id3, "in_progress")
            .await
            .unwrap();
        harness
            .db
            .update_manual_task_status(&id4, "blocked")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        // Summary should show counts in order: pending, in_progress, blocked
        assert!(
            result
                .content
                .contains("Summary: 4 items total \u{2014} 2 pending, 1 in_progress, 1 blocked"),
            "unexpected summary in: {}",
            result.content
        );
        // Note should be present (unfiltered call)
        assert!(
            result
                .content
                .contains("Note: All items are returned in this response."),
            "missing note in: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_tasks_summary_single_status() {
        let harness = TestHarness::new();
        create_test_task(&harness, "Item A", Some("user_request"), None).await;
        create_test_task(&harness, "Item B", Some("user_request"), None).await;
        create_test_task(&harness, "Item C", Some("user_request"), None).await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result
                .content
                .contains("Summary: 3 items total \u{2014} 3 pending"),
            "unexpected summary in: {}",
            result.content
        );
        // Note present for unfiltered
        assert!(result.content.contains("Note: All items are returned"));
    }

    #[tokio::test]
    async fn test_list_tasks_filtered_summary_no_note() {
        let harness = TestHarness::new();
        let id1 = create_test_task(&harness, "Blocked 1", Some("user_request"), None).await;
        let id2 = create_test_task(&harness, "Blocked 2", Some("user_request"), None).await;
        create_test_task(&harness, "Pending 1", Some("user_request"), None).await;

        harness
            .db
            .update_manual_task_status(&id1, "blocked")
            .await
            .unwrap();
        harness
            .db
            .update_manual_task_status(&id2, "blocked")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool
            .execute(serde_json::json!({"status": "blocked"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        // Scoped summary for filtered subset
        assert!(
            result
                .content
                .contains("Summary: 2 items total \u{2014} 2 blocked"),
            "unexpected summary in: {}",
            result.content
        );
        // No note on filtered calls
        assert!(
            !result.content.contains("Note:"),
            "note should not appear on filtered calls: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_tasks_summary_all_statuses() {
        let harness = TestHarness::new();
        let id1 = create_test_task(&harness, "Pending", Some("user_request"), None).await;
        let id2 = create_test_task(&harness, "In Progress", Some("user_request"), None).await;
        let id3 = create_test_task(&harness, "Blocked", Some("user_request"), None).await;
        let id4 = create_test_task(&harness, "Completed", Some("user_request"), None).await;
        let id5 = create_test_task(&harness, "Cancelled", Some("user_request"), None).await;

        // Leave id1 as pending
        let _ = id1;
        harness
            .db
            .update_manual_task_status(&id2, "in_progress")
            .await
            .unwrap();
        harness
            .db
            .update_manual_task_status(&id3, "blocked")
            .await
            .unwrap();
        harness
            .db
            .update_manual_task_status(&id4, "completed")
            .await
            .unwrap();
        harness
            .db
            .update_manual_task_status(&id5, "cancelled")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        // All 5 statuses in declaration order
        assert!(
            result.content.contains(
                "Summary: 5 items total \u{2014} 1 pending, 1 in_progress, 1 blocked, 1 completed, 1 cancelled"
            ),
            "unexpected summary in: {}",
            result.content
        );
    }

    // ===== `type` rendering tests (issue #595) =====

    /// Helper: create a manual task with an explicit type override.
    async fn create_typed_test_task(harness: &TestHarness, label: &str, task_type: &str) -> String {
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
                source: Some("user_request".to_string()),
                metadata: None,
                r#type: Some(task_type.to_string()),
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_list_hides_default_type() {
        // Default-type ("issue") rows should NOT have `type:` in their line —
        // matches the existing pattern of hiding default values.
        let harness = TestHarness::new();
        create_test_task(&harness, "Plain issue", Some("user_request"), None).await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Plain issue"));
        assert!(
            !result.content.contains("type:issue"),
            "default type should be hidden in line: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_shows_milestone_type() {
        let harness = TestHarness::new();
        create_typed_test_task(&harness, "Q2 milestone", "milestone").await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("type:milestone"),
            "milestone type should be visible in line: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_shows_mixed_types() {
        let harness = TestHarness::new();
        create_test_task(&harness, "Plain issue", Some("user_request"), None).await;
        create_typed_test_task(&harness, "Sprint container", "milestone").await;
        create_typed_test_task(&harness, "Q1 project", "project").await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        // milestone + project visible; default issue still hidden
        assert!(result.content.contains("type:milestone"));
        assert!(result.content.contains("type:project"));
        assert!(!result.content.contains("type:issue"));
    }

    #[tokio::test]
    async fn test_list_shows_pid_when_set() {
        let harness = TestHarness::new();
        let id = create_test_task(&harness, "Running task", Some("user_request"), None).await;
        harness.db.set_task_process_id(&id, Some(42)).await.unwrap();
        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result.content.contains("pid:42"),
            "should contain pid annotation: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_hides_pid_when_none() {
        let harness = TestHarness::new();
        create_test_task(&harness, "Normal task", Some("user_request"), None).await;
        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            !result.content.contains("pid:"),
            "should not contain pid when None: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_excludes_non_manual_tasks() {
        let harness = TestHarness::new();
        // Create a non-manual task (callback)
        harness
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
            })
            .await
            .unwrap();
        // Create a manual task
        create_test_task(&harness, "Manual task", Some("user_request"), None).await;

        let ctx = harness.ctx();
        let tool = ListTasksTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Manual task"));
        assert!(!result.content.contains("Callback task"));
        assert!(result.content.contains("Tasks (1)"));
    }
}
