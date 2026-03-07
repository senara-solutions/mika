use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::{debug, warn};

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".to_string(),
            description: "Send a message to the user. Use this in heartbeat and reminder mode \
                to deliver proactive messages. In conversation mode, prefer responding directly."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The message to send to the user"
                    }
                },
                "required": ["text"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let text = input["text"].as_str().unwrap_or("");
        if text.is_empty() {
            return Ok(ToolOutput::error("'text' is required."));
        }
        if text.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'text' too long: {} characters (max: {MAX_INPUT_LEN})",
                text.len()
            )));
        }

        // Persist the outbound message for conversation history
        ctx.db
            .save_message(ctx.session_id, "assistant", text)
            .await?;

        match &ctx.message_sender {
            Some(sender) => {
                sender.send(text).await?;
                debug!("send_message delivered via sender");
                Ok(ToolOutput::success("Message sent."))
            }
            // Intentionally returns success, not error. The message was persisted to the
            // conversation DB (line above), but external delivery was not attempted because
            // no outbound sender is configured. Using ToolOutput::error here would cause
            // Claude to retry the tool call in a loop, since the error is permanent and
            // not fixable by retrying. The warning text gives Claude enough context to
            // inform the user.
            None => {
                warn!("send_message called but no outbound sender configured");
                Ok(ToolOutput::success(
                    "No outbound sender configured — message was NOT delivered. \
                     To enable Telegram delivery, set MIKA_ROUTING_URL and MIKA_INTERNAL_TOKEN.",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::MessageSender;
    use crate::test_utils::test_helpers::TestHarness;
    use std::sync::{Arc, Mutex};

    /// Test sender that captures messages.
    struct MockSender {
        messages: Mutex<Vec<String>>,
    }

    impl MockSender {
        fn new() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
            }
        }

        fn sent(&self) -> Vec<String> {
            self.messages.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessageSender for MockSender {
        async fn send(&self, text: &str) -> Result<()> {
            self.messages.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_send_message_no_sender() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("NOT delivered"));
    }

    #[tokio::test]
    async fn test_send_message_with_sender() {
        let harness = TestHarness::new();
        let mock = Arc::new(MockSender::new());
        let skills_dirty = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            home_dir: std::path::Path::new("/tmp"),
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
        };
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Proactive update"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("sent"));
        assert_eq!(mock.sent(), vec!["Proactive update"]);
    }

    #[tokio::test]
    async fn test_send_message_empty_text() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": ""}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[tokio::test]
    async fn test_send_message_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SendMessageTool;

        let long_text = "x".repeat(MAX_INPUT_LEN + 1);
        let result = tool
            .execute(serde_json::json!({"text": long_text}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("too long"));
    }
}
