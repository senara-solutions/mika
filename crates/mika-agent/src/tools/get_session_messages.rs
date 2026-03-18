use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, is_orchestrator};
use crate::db::format_ts;

pub struct GetSessionMessagesTool;

#[async_trait]
impl Tool for GetSessionMessagesTool {
    fn name(&self) -> &str {
        "get_session_messages"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_session_messages".to_string(),
            description: "Retrieve messages from a past conversation session. Useful for replaying or summarizing old conversations. Non-orchestrator agents can only access their own sessions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "The session ID to retrieve messages from."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of messages to return (default: 50, max: 200)."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of messages to skip for pagination (default: 0)."
                    }
                },
                "required": ["session_id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let session_id = input["session_id"].as_str().unwrap_or("");
        if session_id.is_empty() {
            return Ok(ToolOutput::error("'session_id' is required."));
        }
        if session_id.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'session_id' too long: {} characters (max: {MAX_INPUT_LEN})",
                session_id.len()
            )));
        }

        let limit = input["limit"].as_u64().unwrap_or(50).min(200) as u32;
        let offset = input["offset"].as_u64().unwrap_or(0) as u32;

        // Verify the session exists and check ownership
        let session = ctx.db.get_session(session_id).await?;
        let session = match session {
            Some(s) => s,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Session '{session_id}' not found."
                )));
            }
        };

        // Non-orchestrator agents can only access their own sessions
        let agent_id = ctx.db.agent_id();
        if !is_orchestrator(ctx.home_dir, agent_id) && session.agent_id != agent_id {
            return Ok(ToolOutput::error(
                "Access denied: you can only view your own sessions.",
            ));
        }

        let messages = ctx
            .db
            .load_session_messages_paginated(session_id, limit, offset)
            .await?;

        if messages.is_empty() {
            return Ok(ToolOutput::success(format!(
                "No messages in session '{session_id}' (offset={offset})."
            )));
        }

        let total = ctx.db.count_session_messages(session_id).await?;
        let mut output = format!(
            "Session '{}' (agent: {}, channel: {}, showing {}-{} of {}):\n",
            session_id,
            session.agent_id,
            session.channel_type,
            offset + 1,
            (offset as u64 + messages.len() as u64).min(total),
            total,
        );

        for msg in &messages {
            let ts = format_ts(&msg.created_at);
            // Truncate long messages for readability
            let content = if msg.content.len() > 500 {
                format!("{}... ({} chars)", &msg.content[..500], msg.content.len())
            } else {
                msg.content.clone()
            };
            output.push_str(&format!("[{}] {}: {}\n", ts, msg.role, content));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_get_session_messages_missing_session_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'session_id' is required"));
    }

    #[tokio::test]
    async fn test_get_session_messages_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let result = tool
            .execute(serde_json::json!({"session_id": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_get_session_messages_empty_session() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let result = tool
            .execute(serde_json::json!({"session_id": "test-session"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No messages"));
    }

    #[tokio::test]
    async fn test_get_session_messages_with_data() {
        let harness = TestHarness::new();
        harness
            .db
            .save_message("test-session", "user", "Hello", None)
            .await
            .unwrap();
        harness
            .db
            .save_message("test-session", "assistant", "Hi there", None)
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let result = tool
            .execute(serde_json::json!({"session_id": "test-session"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Hello"));
        assert!(result.content.contains("Hi there"));
        assert!(result.content.contains("user:"));
        assert!(result.content.contains("assistant:"));
        assert!(result.content.contains("of 2"));
    }

    #[tokio::test]
    async fn test_get_session_messages_pagination() {
        let harness = TestHarness::new();
        for i in 0..5 {
            harness
                .db
                .save_message("test-session", "user", &format!("Message {i}"), None)
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        // First page
        let result = tool
            .execute(
                serde_json::json!({"session_id": "test-session", "limit": 2}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 1-2 of 5"));
        assert!(result.content.contains("Message 0"));
        assert!(result.content.contains("Message 1"));

        // Second page
        let result = tool
            .execute(
                serde_json::json!({"session_id": "test-session", "limit": 2, "offset": 2}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("showing 3-4 of 5"));
    }

    #[tokio::test]
    async fn test_get_session_messages_access_denied_for_other_agent() {
        let harness = TestHarness::with_agent("helper");
        // "test-session" belongs to "helper" agent — but let's try accessing
        // a session that belongs to a different agent. We need to create a session
        // for another agent first.
        harness
            .db
            .with_agent("mika")
            .create_session("mika-session", "mika", "cli")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let result = tool
            .execute(serde_json::json!({"session_id": "mika-session"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Access denied"));
    }

    #[tokio::test]
    async fn test_get_session_messages_session_id_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetSessionMessagesTool;

        let long_id = "x".repeat(MAX_INPUT_LEN + 1);
        let result = tool
            .execute(serde_json::json!({"session_id": long_id}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too long"));
    }
}
