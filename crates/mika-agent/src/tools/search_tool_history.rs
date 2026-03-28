use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, is_orchestrator};
use crate::db::ToolCallFilters;

/// Maximum characters per input/output field in results.
const FIELD_TRUNCATE_LEN: usize = 500;
/// Maximum total output size in bytes.
const MAX_OUTPUT_BYTES: usize = 10_000;
/// Default result limit.
const DEFAULT_LIMIT: u32 = 20;
/// Maximum result limit.
const MAX_LIMIT: u32 = 50;

pub struct SearchToolHistoryTool;

#[async_trait]
impl Tool for SearchToolHistoryTool {
    fn name(&self) -> &str {
        "search_tool_history"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_tool_history".to_string(),
            description: "Search past tool call history across sessions. Returns tool names, \
                truncated input/output, success status, and timing. Useful for recalling prior \
                tool results without re-running external calls. Tool call history is retained \
                for 30 days. Non-orchestrator agents are scoped to their own tool calls."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": "Filter by exact tool name (e.g., 'search_memory', 'web_search')."
                    },
                    "keyword": {
                        "type": "string",
                        "description": "Search for a keyword in tool input or output fields."
                    },
                    "from": {
                        "type": "string",
                        "description": "Start time filter in ISO 8601 format (e.g., '2026-03-27T00:00:00Z')."
                    },
                    "to": {
                        "type": "string",
                        "description": "End time filter in ISO 8601 format (e.g., '2026-03-28T00:00:00Z')."
                    },
                    "success": {
                        "type": "boolean",
                        "description": "Filter by success (true) or failure (false)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 20, max: 50)."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Validate optional string inputs
        for field in &["tool_name", "keyword", "from", "to"] {
            if let Some(v) = input[field].as_str()
                && v.len() > MAX_INPUT_LEN
            {
                return Ok(ToolOutput::error(format!(
                    "'{field}' too long: {} characters (max: {MAX_INPUT_LEN})",
                    v.len()
                )));
            }
        }

        // Validate keyword is not empty if provided
        if let Some(kw) = input["keyword"].as_str() {
            let trimmed = kw.trim();
            if trimmed.is_empty() {
                return Ok(ToolOutput::error("keyword must not be empty.".to_string()));
            }
        }

        let limit = input["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_LIMIT as u64)
            .min(MAX_LIMIT as u64) as u32;

        let agent_id = ctx.db.agent_id();
        let is_orch = is_orchestrator(ctx.home_dir, agent_id);

        let filters = ToolCallFilters {
            agent_id: if is_orch {
                None
            } else {
                Some(agent_id.to_string())
            },
            session_id: None,
            trace_id: None,
            tool_name: input["tool_name"].as_str().map(|s| s.to_string()),
            success: input["success"].as_bool(),
            from: input["from"].as_str().map(|s| s.to_string()),
            to: input["to"].as_str().map(|s| s.to_string()),
            keyword: input["keyword"].as_str().map(|s| s.trim().to_string()),
        };

        let (rows, total) = ctx.db.query_tool_calls(filters, 1, limit).await?;

        if rows.is_empty() {
            return Ok(ToolOutput::success("No tool call history found."));
        }

        let mut output = format!(
            "Tool call history ({} result(s){})\n\n",
            rows.len(),
            if total > rows.len() as u64 {
                format!(", {} total matches", total)
            } else {
                String::new()
            }
        );

        for row in &rows {
            // Check total output size budget
            if output.len() >= MAX_OUTPUT_BYTES {
                output.push_str(&format!(
                    "\n[Output truncated at {}KB — {} more result(s) not shown]\n",
                    MAX_OUTPUT_BYTES / 1000,
                    total as usize - rows.len().min(total as usize)
                ));
                break;
            }

            let status = if row.success { "OK" } else { "FAILED" };
            let tool_src = if row.tool_source == "builtin" {
                String::new()
            } else {
                format!(" [{}]", row.tool_source)
            };
            let skill = row
                .skill_name
                .as_deref()
                .map(|s| format!(" skill:{s}"))
                .unwrap_or_default();

            output.push_str(&format!(
                "--- {} | {}{}{} | {} | {}ms\n",
                row.created_at, row.tool_name, tool_src, skill, status, row.latency_ms
            ));

            if let Some(ref inp) = row.input {
                let truncated = truncate_field(inp, FIELD_TRUNCATE_LEN);
                output.push_str(&format!("  Input: {truncated}\n"));
            }
            if let Some(ref out) = row.output {
                let truncated = truncate_field(out, FIELD_TRUNCATE_LEN);
                output.push_str(&format!("  Output: {truncated}\n"));
            }
            if let Some(ref err) = row.error_message {
                let truncated = truncate_field(err, FIELD_TRUNCATE_LEN);
                output.push_str(&format!("  Error: {truncated}\n"));
            }
            output.push('\n');
        }

        Ok(ToolOutput::success(output))
    }
}

/// Truncate a string to a maximum length with a suffix indicator.
fn truncate_field(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    // Find a valid UTF-8 boundary
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... [truncated]", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_search_empty_table() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No tool call history found"));
    }

    #[tokio::test]
    async fn test_search_returns_results() {
        let harness = TestHarness::new();
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                Some("trace-1"),
                None,
                0,
                "search_memory",
                "builtin",
                None,
                Some("query: coffee"),
                Some("Found 3 results about coffee preferences"),
                true,
                false,
                42,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("search_memory"));
        assert!(result.content.contains("coffee"));
        assert!(result.content.contains("OK"));
        assert!(result.content.contains("42ms"));
    }

    #[tokio::test]
    async fn test_search_by_tool_name() {
        let harness = TestHarness::new();
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                None,
                None,
                0,
                "search_memory",
                "builtin",
                None,
                Some("q1"),
                Some("r1"),
                true,
                false,
                10,
                None,
            )
            .await
            .unwrap();
        harness
            .db
            .save_tool_call(
                "tc-2",
                "test-session",
                None,
                None,
                0,
                "store_fact",
                "builtin",
                None,
                Some("q2"),
                Some("r2"),
                true,
                false,
                20,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool
            .execute(serde_json::json!({"tool_name": "search_memory"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("search_memory"));
        assert!(!result.content.contains("store_fact"));
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let harness = TestHarness::new();
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                None,
                None,
                0,
                "search_memory",
                "builtin",
                None,
                Some("query: coffee preferences"),
                Some("likes espresso"),
                true,
                false,
                10,
                None,
            )
            .await
            .unwrap();
        harness
            .db
            .save_tool_call(
                "tc-2",
                "test-session",
                None,
                None,
                0,
                "search_memory",
                "builtin",
                None,
                Some("query: weather"),
                Some("sunny today"),
                true,
                false,
                20,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool
            .execute(serde_json::json!({"keyword": "coffee"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("espresso"));
        assert!(!result.content.contains("weather"));
    }

    #[tokio::test]
    async fn test_search_by_success_filter() {
        let harness = TestHarness::new();
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                None,
                None,
                0,
                "web_search",
                "builtin",
                None,
                Some("q"),
                Some("ok"),
                true,
                false,
                10,
                None,
            )
            .await
            .unwrap();
        harness
            .db
            .save_tool_call(
                "tc-2",
                "test-session",
                None,
                None,
                0,
                "web_search",
                "builtin",
                None,
                Some("q"),
                None,
                false,
                false,
                10,
                Some("timeout"),
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool
            .execute(serde_json::json!({"success": false}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("FAILED"));
        assert!(result.content.contains("timeout"));
        // Should not contain the successful one's output
        assert!(!result.content.contains("Output: ok"));
    }

    #[tokio::test]
    async fn test_search_limit_capped() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        // limit: 9999 should be capped to MAX_LIMIT (50)
        let result = tool
            .execute(serde_json::json!({"limit": 9999}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_search_input_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let long_string = "x".repeat(MAX_INPUT_LEN + 1);
        let result = tool
            .execute(serde_json::json!({"keyword": long_string}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too long"));
    }

    #[tokio::test]
    async fn test_search_empty_keyword_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool
            .execute(serde_json::json!({"keyword": "  "}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("must not be empty"));
    }

    #[tokio::test]
    async fn test_search_output_truncation() {
        let harness = TestHarness::new();
        let long_output = "x".repeat(1000);
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                None,
                None,
                0,
                "test_tool",
                "builtin",
                None,
                Some(&long_output),
                Some(&long_output),
                true,
                false,
                10,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[truncated]"));
        // The full 1000-char string should not appear
        assert!(!result.content.contains(&long_output));
    }

    #[test]
    fn test_truncate_field() {
        assert_eq!(truncate_field("short", 10), "short");
        assert_eq!(truncate_field("hello world", 5), "hello... [truncated]");
    }

    #[tokio::test]
    async fn test_search_agent_scoping() {
        // Non-orchestrator agent should only see own tool calls
        let harness = TestHarness::with_agent("researcher");
        harness
            .db
            .save_tool_call(
                "tc-1",
                "test-session",
                None,
                None,
                0,
                "tool_a",
                "builtin",
                None,
                Some("input"),
                Some("output"),
                true,
                false,
                10,
                None,
            )
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = SearchToolHistoryTool;

        // Should see own tool calls
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("tool_a"));
    }
}
