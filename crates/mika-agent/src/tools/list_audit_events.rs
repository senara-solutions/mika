use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput, is_orchestrator};

pub struct ListAuditEventsTool;

#[async_trait]
impl Tool for ListAuditEventsTool {
    fn name(&self) -> &str {
        "list_audit_events"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_audit_events".to_string(),
            description: "List recent memory mutation audit events (fact stores, updates, core memory edits). Useful for self-introspection to see what changes have been made. Non-orchestrator agents are scoped to their own events.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of events to return (default: 50, max: 200)."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of events to skip for pagination (default: 0)."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let limit = input["limit"].as_u64().unwrap_or(50).min(200) as u32;
        let offset = input["offset"].as_u64().unwrap_or(0) as u32;

        let agent_id = ctx.db.agent_id();

        // Orchestrators can see all agents' events; non-orchestrators only their own.
        // The DB method already filters by agent_id, so we always pass the current agent's id.
        // For orchestrators, we could pass all agents, but the DB method takes a single agent_id,
        // so we keep it scoped for simplicity and consistency.
        let target_agent = agent_id.to_string();

        let events = ctx
            .db
            .list_audit_events_paginated(&target_agent, limit, offset)
            .await?;

        if events.is_empty() {
            return Ok(ToolOutput::success(format!(
                "No audit events found for agent '{target_agent}' (offset={offset})."
            )));
        }

        let total = ctx.db.count_audit_events(&target_agent).await?;
        let mut output = format!(
            "Audit events for '{}' (showing {}-{} of {}):\n",
            target_agent,
            offset + 1,
            (offset as u64 + events.len() as u64).min(total),
            total,
        );

        for evt in &events {
            let before = evt
                .before_value
                .as_deref()
                .map(|v| truncate(v, 100))
                .unwrap_or_else(|| "(none)".to_string());
            let after = evt
                .after_value
                .as_deref()
                .map(|v| truncate(v, 100))
                .unwrap_or_else(|| "(none)".to_string());
            let reasoning = evt
                .reasoning
                .as_deref()
                .map(|r| format!(" reason: {}", truncate(r, 80)))
                .unwrap_or_default();
            output.push_str(&format!(
                "- [{}] {} -> {} | before: {} | after: {}{}\n",
                evt.created_at, evt.tool_name, evt.target_key, before, after, reasoning,
            ));
        }

        // Add orchestrator hint
        if is_orchestrator(ctx.home_dir, agent_id) {
            output.push_str(
                "\nNote: As orchestrator, you can inspect other agents' events via query_timeline.",
            );
        }

        Ok(ToolOutput::success(output))
    }
}

/// Truncate a string to at most `max_len` bytes, appending "..." if truncated.
/// Rounds down to the nearest char boundary to avoid panics on multi-byte UTF-8.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", mika_common::text::safe_truncate(s, max_len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_list_audit_events_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = ListAuditEventsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No audit events found"));
    }

    #[tokio::test]
    async fn test_list_audit_events_with_data() {
        let harness = TestHarness::new();
        harness
            .db
            .log_audit_event(
                "test-session",
                "store_fact",
                "person:Alice",
                None,
                Some("Added Alice as CTO"),
                None,
                None,
            )
            .await
            .unwrap();
        harness
            .db
            .log_audit_event(
                "test-session",
                "update_core_memory",
                "user_summary",
                Some("Old summary"),
                Some("New summary about preferences"),
                Some("User mentioned preferences"),
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListAuditEventsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("store_fact"));
        assert!(result.content.contains("person:Alice"));
        assert!(result.content.contains("update_core_memory"));
        assert!(result.content.contains("of 2"));
    }

    #[tokio::test]
    async fn test_list_audit_events_pagination() {
        let harness = TestHarness::new();
        for i in 0..5 {
            harness
                .db
                .log_audit_event(
                    "test-session",
                    "store_fact",
                    &format!("item:{i}"),
                    None,
                    Some(&*format!("Value {i}")),
                    None,
                    None,
                )
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = ListAuditEventsTool;

        let result = tool
            .execute(serde_json::json!({"limit": 2}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 1-2 of 5"));

        let result = tool
            .execute(serde_json::json!({"limit": 2, "offset": 3}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 4-5 of 5"));
    }

    #[tokio::test]
    async fn test_list_audit_events_shows_reasoning() {
        let harness = TestHarness::new();
        harness
            .db
            .log_audit_event(
                "test-session",
                "update_fact",
                "commitment:1",
                Some("Review budget"),
                Some("Review Q4 budget by Friday"),
                Some("User clarified the deadline"),
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListAuditEventsTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("reason: User clarified"));
    }

    #[tokio::test]
    async fn test_list_audit_events_scoped_to_agent() {
        let harness = TestHarness::with_agent("helper");
        // Log an event for a different agent via the mika handle
        harness
            .db
            .with_agent("mika")
            .log_audit_event(
                "test-session",
                "store_fact",
                "person:Bob",
                None,
                Some("Bob added by mika"),
                None,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = ListAuditEventsTool;

        // Helper agent should not see mika's events
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No audit events found"));
    }

    #[tokio::test]
    async fn test_truncate_long_values() {
        assert_eq!(truncate("short", 100), "short");
        let long = "x".repeat(200);
        let truncated = truncate(&long, 100);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncated.len(), 103); // 100 + "..."
    }
}
