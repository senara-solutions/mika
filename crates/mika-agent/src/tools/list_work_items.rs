use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_unix_ts;

pub struct ListWorkItemsTool;

#[async_trait]
impl Tool for ListWorkItemsTool {
    fn name(&self) -> &str {
        "list_work_items"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_work_items".to_string(),
            description: "List tracked work items with optional filtering by status and source. \
                Returns up to 50 work items, ordered by creation date (newest first). \
                Check this before creating new work items to avoid duplicates."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Filter by status: pending, in_progress, blocked, completed, cancelled"
                    },
                    "source": {
                        "type": "string",
                        "description": "Filter by source: user_request, github_issue, team_run, self_dev"
                    },
                    "include_children": {
                        "type": "boolean",
                        "description": "Include child task count for each work item (default: false)"
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

        let items = ctx
            .db
            .list_manual_tasks(status, source, include_children)
            .await?;

        if items.is_empty() {
            let mut msg = "No work items found".to_string();
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
        lines.push(format!("Work items ({}):\n", items.len()));

        for (task, child_count) in &items {
            let created = format_unix_ts(task.created_at);
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
            let children = child_count
                .map(|c| format!(" children:{c}"))
                .unwrap_or_default();

            lines.push(format!(
                "- [{status}] {id} {label} (created:{created}{ref_url}{src}{children})",
                status = task.status,
                id = task.id,
                label = task.label,
            ));
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

    async fn create_work_item(
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
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_list_work_items_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No work items found"));
    }

    #[tokio::test]
    async fn test_list_work_items_basic() {
        let harness = TestHarness::new();
        create_work_item(&harness, "Item A", Some("user_request"), None).await;
        create_work_item(
            &harness,
            "Item B",
            Some("github_issue"),
            Some("https://github.com/org/repo/issues/1"),
        )
        .await;
        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Item A"));
        assert!(result.content.contains("Item B"));
        assert!(result.content.contains("Work items (2)"));
    }

    #[tokio::test]
    async fn test_list_work_items_filter_by_status() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Active item", Some("user_request"), None).await;
        create_work_item(&harness, "Pending item", Some("user_request"), None).await;

        // Move one to in_progress
        harness
            .db
            .update_manual_task_status(&id, "in_progress")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

        let result = tool
            .execute(serde_json::json!({"status": "in_progress"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Active item"));
        assert!(!result.content.contains("Pending item"));
    }

    #[tokio::test]
    async fn test_list_work_items_filter_by_source() {
        let harness = TestHarness::new();
        create_work_item(&harness, "User item", Some("user_request"), None).await;
        create_work_item(&harness, "GH item", Some("github_issue"), None).await;
        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

        let result = tool
            .execute(serde_json::json!({"source": "github_issue"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("GH item"));
        assert!(!result.content.contains("User item"));
    }

    #[tokio::test]
    async fn test_list_work_items_with_children() {
        let harness = TestHarness::new();
        let parent_id = create_work_item(&harness, "Parent item", Some("user_request"), None).await;
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
            })
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

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
            })
            .await
            .unwrap();
        // Create a manual task
        create_work_item(&harness, "Work item", Some("user_request"), None).await;

        let ctx = harness.ctx();
        let tool = ListWorkItemsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Work item"));
        assert!(!result.content.contains("Callback task"));
        assert!(result.content.contains("Work items (1)"));
    }
}
