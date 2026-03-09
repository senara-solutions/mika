use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, is_orchestrator};
use crate::db::{TimelineFilters, format_unix_ts};

pub struct QueryTimelineTool;

#[async_trait]
impl Tool for QueryTimelineTool {
    fn name(&self) -> &str {
        "query_timeline"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "query_timeline".to_string(),
            description: "Query the unified timeline of events across all subsystems (messages, audit events, tasks). Returns recent activity sorted by time. Non-orchestrator agents are scoped to their own agent_id.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_type": {
                        "type": "string",
                        "description": "Optional filter by event type: 'message', 'audit', or 'task'.",
                        "enum": ["message", "audit", "task"]
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional filter by session ID."
                    },
                    "trace_id": {
                        "type": "string",
                        "description": "Optional filter by trace ID for cross-subsystem correlation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of events to return (default: 50, max: 200)."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Validate optional string inputs
        for field in &["event_type", "session_id", "trace_id"] {
            if let Some(v) = input[field].as_str()
                && v.len() > MAX_INPUT_LEN
            {
                return Ok(ToolOutput::error(format!(
                    "'{field}' too long: {} characters (max: {MAX_INPUT_LEN})",
                    v.len()
                )));
            }
        }

        let limit = input["limit"].as_u64().unwrap_or(50).min(200) as u32;

        let agent_id = ctx.db.agent_id();
        let is_orch = is_orchestrator(ctx.home_dir, agent_id);

        let filters = TimelineFilters {
            agent_id: if is_orch {
                None
            } else {
                Some(agent_id.to_string())
            },
            event_type: input["event_type"].as_str().map(|s| s.to_string()),
            trace_id: input["trace_id"].as_str().map(|s| s.to_string()),
            session_id: input["session_id"].as_str().map(|s| s.to_string()),
            from: None,
            to: None,
        };

        let rows = ctx.db.query_timeline(filters, limit, 0).await?;

        if rows.is_empty() {
            return Ok(ToolOutput::success("No timeline events found."));
        }

        let mut output = format!("Timeline ({} event(s)):\n", rows.len());
        for row in &rows {
            let ts = format_unix_ts(row.created_at);
            let trace = row
                .trace_id
                .as_deref()
                .map(|t| &t[..8.min(t.len())])
                .unwrap_or("-");
            let summary = row.summary.as_deref().unwrap_or("");
            output.push_str(&format!(
                "- [{}] {} {}/{} agent={} {}\n",
                ts, trace, row.event_type, row.event_subtype, row.agent_id, summary
            ));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_query_timeline_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No timeline events found"));
    }

    #[tokio::test]
    async fn test_query_timeline_with_messages() {
        let harness = TestHarness::new();
        harness
            .db
            .save_message("test-session", "user", "Hello world", None)
            .await
            .unwrap();
        harness
            .db
            .save_message("test-session", "assistant", "Hi there", None)
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        let result = tool
            .execute(serde_json::json!({"event_type": "message"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Timeline"));
        assert!(result.content.contains("message"));
    }

    #[tokio::test]
    async fn test_query_timeline_with_audit_events() {
        let harness = TestHarness::new();
        harness
            .db
            .log_audit_event(
                "test-session",
                "store_fact",
                "person:Alice",
                None,
                "Added Alice",
                None,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        let result = tool
            .execute(serde_json::json!({"event_type": "audit"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("audit"));
    }

    #[tokio::test]
    async fn test_query_timeline_limit() {
        let harness = TestHarness::new();
        // Insert a few messages
        for i in 0..5 {
            harness
                .db
                .save_message(
                    "test-session",
                    "user",
                    &format!("Message {i}"),
                    None,
                )
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        let result = tool
            .execute(serde_json::json!({"limit": 2}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("2 event(s)"));
    }

    #[tokio::test]
    async fn test_query_timeline_limit_capped_at_200() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        // Should not error even with a large limit — capped to 200
        let result = tool
            .execute(serde_json::json!({"limit": 9999}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_query_timeline_input_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = QueryTimelineTool;

        let long_string = "x".repeat(MAX_INPUT_LEN + 1);
        let result = tool
            .execute(serde_json::json!({"trace_id": long_string}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too long"));
    }
}
