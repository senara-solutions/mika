use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::info;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct SendMessageTool;

#[async_trait(?Send)]
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
                    },
                    "urgency": {
                        "type": "string",
                        "enum": ["high", "normal", "low"],
                        "description": "Message priority. Currently all deliver immediately."
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

        match &ctx.message_sender {
            Some(sender) => {
                sender.send(text).await?;
                Ok(ToolOutput::success("Message sent."))
            }
            None => {
                info!(text, "send_message (CLI mode, no sender configured)");
                Ok(ToolOutput::success("Message delivered (CLI)."))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::MessageSender;
    use crate::test_utils::test_helpers::{test_ctx, test_db};
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

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

    #[async_trait(?Send)]
    impl MessageSender for MockSender {
        async fn send(&self, text: &str) -> Result<()> {
            self.messages.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_send_message_no_sender() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("CLI"));
    }

    #[tokio::test]
    async fn test_send_message_with_sender() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let mock = MockSender::new();
        let ctx = crate::tools::ToolContext {
            db: &db,
            session_id: "test",
            home_dir: std::path::Path::new("/tmp"),
            core_memory_edit_count: &counter,
            is_onboarding: false,
            message_sender: Some(&mock),
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
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
