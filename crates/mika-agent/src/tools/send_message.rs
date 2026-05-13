use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::{debug, error, warn};

use crate::messaging::SendOutcome;

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
            description: "Send a message to the user via Telegram. Use this when you need to \
                deliver a message directly to the user — especially in heartbeat, reminder, \
                and delegation mode. When delegated a task that involves sending a message, \
                you MUST use this tool to deliver it. In conversation mode, prefer responding \
                directly unless the task specifically requires sending a separate message."
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

        // Strip any internal metadata tags the LLM may have echoed
        let cleaned = mika_common::llm::strip_internal_tags(text);
        if cleaned.is_empty() {
            return Ok(ToolOutput::success("Message was empty after processing."));
        }

        // Persist the outbound message for conversation history
        ctx.db
            .save_message(ctx.session_id, "assistant", &cleaned, Some(ctx.trace_id))
            .await?;

        match &ctx.message_sender {
            Some(sender) => {
                debug!("send_message: delivering via configured sender");
                match sender.send(&cleaned).await {
                    Ok(SendOutcome::Delivered) => {
                        debug!("send_message: delivered successfully");
                        Ok(ToolOutput::success("Message sent."))
                    }
                    Ok(SendOutcome::Failed { reason }) => {
                        warn!(reason = %reason, "send_message: delivery failed");
                        Ok(ToolOutput::error(format!(
                            "Message delivery failed: {reason}"
                        )))
                    }
                    // Intentionally returns success, not error. chat_id == 0 is a
                    // permanent session condition (GitHub webhook / non-Telegram
                    // channel), not a transient failure. Using ToolOutput::error
                    // would cause Claude to retry in a loop. The message tells the
                    // LLM to use channel-appropriate tools instead.
                    Ok(SendOutcome::NoChannel) => {
                        error!(
                            trace_id = %ctx.trace_id,
                            session_id = %ctx.session_id,
                            "send_message_nochannel: tool returned success but message was NOT delivered — chat_id=0"
                        );
                        Ok(ToolOutput::success(
                            "No reply channel for this session (chat_id is zero). \
                             The user cannot receive messages via send_message. \
                             Use channel-appropriate tools (e.g., run_gh for GitHub) \
                             to deliver your response.",
                        ))
                    }
                    Err(e) => {
                        warn!(error = %e, "send_message: sender error");
                        Ok(ToolOutput::error(format!("Message delivery error: {e}")))
                    }
                }
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
    use crate::messaging::{MessageSender, SendOutcome};
    use crate::test_utils::test_helpers::TestHarness;
    use std::sync::{Arc, Mutex};

    /// Test sender that captures messages and returns a configurable outcome.
    struct MockSender {
        messages: Mutex<Vec<String>>,
        outcome: Mutex<Option<SendOutcome>>,
        /// When set, `send()` returns `Err` instead of `Ok(outcome)`.
        infra_error: Mutex<Option<String>>,
    }

    impl MockSender {
        /// Create a sender that always succeeds.
        fn new() -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                outcome: Mutex::new(None),
                infra_error: Mutex::new(None),
            }
        }

        /// Create a sender that returns a specific `SendOutcome`.
        fn with_outcome(outcome: SendOutcome) -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                outcome: Mutex::new(Some(outcome)),
                infra_error: Mutex::new(None),
            }
        }

        /// Create a sender that returns an infrastructure error.
        fn with_error(msg: impl Into<String>) -> Self {
            Self {
                messages: Mutex::new(Vec::new()),
                outcome: Mutex::new(None),
                infra_error: Mutex::new(Some(msg.into())),
            }
        }

        fn sent(&self) -> Vec<String> {
            self.messages.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessageSender for MockSender {
        async fn send(&self, text: &str) -> Result<SendOutcome> {
            self.messages.lock().unwrap().push(text.to_string());
            if let Some(err) = self.infra_error.lock().unwrap().as_ref() {
                return Err(anyhow::anyhow!("{}", err));
            }
            match self.outcome.lock().unwrap().clone() {
                Some(outcome) => Ok(outcome),
                None => Ok(SendOutcome::Delivered),
            }
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
        let pr_review_posted = std::sync::atomic::AtomicBool::new(false);
        let tool_arg_suffix_rejected = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new("/tmp"),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &tool_arg_suffix_rejected,
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

    #[tokio::test]
    async fn test_send_message_gateway_failure() {
        let harness = TestHarness::new();
        let mock = Arc::new(MockSender::with_outcome(SendOutcome::Failed {
            reason: "gateway /send returned 502 Bad Gateway".to_string(),
        }));
        let skills_dirty = std::sync::atomic::AtomicBool::new(false);
        let pr_review_posted = std::sync::atomic::AtomicBool::new(false);
        let tool_arg_suffix_rejected = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new("/tmp"),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &tool_arg_suffix_rejected,
        };
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(
            result.is_error,
            "expected is_error=true for gateway failure"
        );
        assert!(
            result.content.contains("502 Bad Gateway"),
            "expected status in error: {}",
            result.content
        );
        assert!(
            result.content.contains("delivery failed"),
            "expected 'delivery failed' in error: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_send_message_infra_error() {
        let harness = TestHarness::new();
        let mock = Arc::new(MockSender::with_error(
            "chat_id not configured — no Telegram pairing yet",
        ));
        let skills_dirty = std::sync::atomic::AtomicBool::new(false);
        let pr_review_posted = std::sync::atomic::AtomicBool::new(false);
        let tool_arg_suffix_rejected = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new("/tmp"),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &tool_arg_suffix_rejected,
        };
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error, "expected is_error=true for infra error");
        assert!(
            result.content.contains("delivery error"),
            "expected 'delivery error' in error: {}",
            result.content
        );
        assert!(
            result.content.contains("chat_id not configured"),
            "expected error details: {}",
            result.content
        );
    }

    /// NoChannel outcome returns success (not error) with actionable text (#650).
    ///
    /// Level 1 regression guard (mika#1090): NoChannel MUST return ToolOutput::success,
    /// not ToolOutput::error. Returning error causes LLM retry loops because chat_id=0
    /// is a permanent session condition. Level 3 (rejecting the call) would change this
    /// intentionally — that's a separate ticket with retry-semantic coupling analysis.
    #[tokio::test]
    async fn test_send_message_no_channel() {
        let harness = TestHarness::new();
        let mock = Arc::new(MockSender::with_outcome(SendOutcome::NoChannel));
        let skills_dirty = std::sync::atomic::AtomicBool::new(false);
        let pr_review_posted = std::sync::atomic::AtomicBool::new(false);
        let tool_arg_suffix_rejected = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new("/tmp"),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &tool_arg_suffix_rejected,
        };
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "NoChannel should return success (not error) to avoid LLM retry loops"
        );
        assert!(
            result.content.contains("No reply channel"),
            "expected 'No reply channel' in output: {}",
            result.content
        );
        assert!(
            result.content.contains("run_gh"),
            "expected tool redirect hint in output: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_send_message_connection_error() {
        let harness = TestHarness::new();
        let mock = Arc::new(MockSender::with_outcome(SendOutcome::Failed {
            reason: "gateway unreachable (connection error): connection refused".to_string(),
        }));
        let skills_dirty = std::sync::atomic::AtomicBool::new(false);
        let pr_review_posted = std::sync::atomic::AtomicBool::new(false);
        let tool_arg_suffix_rejected = std::sync::atomic::AtomicBool::new(false);
        let ctx = crate::tools::ToolContext {
            db: &harness.db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new("/tmp"),
            global_home_dir: None,
            core_memory_edit_count: &harness.counter,
            is_onboarding: false,
            message_sender: Some(mock.clone()),
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            skills_dirty: &skills_dirty,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &pr_review_posted,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &tool_arg_suffix_rejected,
        };
        let tool = SendMessageTool;

        let result = tool
            .execute(serde_json::json!({"text": "Hello!"}), &ctx)
            .await
            .unwrap();
        assert!(
            result.is_error,
            "expected is_error=true for connection error"
        );
        assert!(
            result.content.contains("connection"),
            "expected connection info in error: {}",
            result.content
        );
    }
}
