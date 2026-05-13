use anyhow::{Result, anyhow};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;
use tracing::{error, warn};

use crate::async_db::AsyncDatabase;

/// Outcome of a message delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Message was delivered successfully (gateway returned 2xx).
    Delivered,
    /// Delivery failed after retries. The message was saved to `failed_sends`
    /// for later flush, but the agent should treat this as a failure.
    Failed {
        /// Human-readable reason including HTTP status or network error classification.
        reason: String,
    },
    /// No reply channel available for this session. `chat_id == 0` is the
    /// documented sentinel for GitHub webhook and other non-Telegram channels
    /// where the agent should use channel-appropriate tools (e.g., `run_gh`)
    /// instead of `send_message`. This is a permanent session condition, not a
    /// transient error — callers should not retry or save to `failed_sends`.
    NoChannel,
}

/// Trait for sending outbound messages to the user.
/// CLI mode uses the fallback in send_message tool; HTTP mode will POST to the gateway.
///
/// **Text-only outbound:** This trait accepts only `text: &str` — agent responses
/// delivered to users are always plain text. Tool-produced images (e.g., from exec
/// handler scripts returning `__mika_v1` envelopes, or the file-reader skill) are
/// included in the Claude API `tool_result` content blocks for the LLM's visual
/// analysis, but are never forwarded to the end user through this trait. If outbound
/// image delivery is needed in the future, the `send` signature and the gateway
/// `/send` payload would need to be extended.
///
/// Send + Sync bounds allow `Arc<dyn MessageSender>` in AppState and ToolContext.
#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, text: &str) -> Result<SendOutcome>;
}

/// Sends outbound messages by POSTing to the gateway's /send endpoint.
///
/// On failure, retries once after 2s. If both attempts fail, saves to
/// `failed_sends` table for later flush and returns `Ok(SendOutcome::Failed)`
/// so the caller can surface the delivery failure to the LLM.
pub struct GatewayMessageSender {
    client: reqwest::Client,
    gateway_url: String,
    internal_token: SecretString,
    db: AsyncDatabase,
    request_id: Option<String>,
    agent_name: Option<String>,
    /// Explicit chat_id override. When set, skips the DB lookup in `send()`.
    /// Used by delegated agents whose agent-scoped `customer_config` doesn't
    /// contain the chat_id (it's stored under the orchestrator's agent_id).
    chat_id: Option<i64>,
}

impl GatewayMessageSender {
    pub fn new(
        gateway_url: String,
        internal_token: SecretString,
        db: AsyncDatabase,
        client: reqwest::Client,
        request_id: Option<String>,
        agent_name: Option<String>,
        chat_id: Option<i64>,
    ) -> Self {
        Self {
            client,
            gateway_url,
            internal_token,
            db,
            request_id,
            agent_name,
            chat_id,
        }
    }

    async fn resolve_chat_id(&self) -> Result<i64> {
        match self.chat_id {
            Some(id) => Ok(id),
            None => self
                .db
                .get_customer_config("chat_id")
                .await?
                .ok_or_else(|| anyhow!("chat_id not configured — no Telegram pairing yet"))?
                .parse::<i64>()
                .map_err(|e| anyhow!("invalid chat_id: {e}")),
        }
    }

    async fn try_send(&self, payload: &serde_json::Value) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/send", self.gateway_url))
            .bearer_auth(self.internal_token.expose_secret())
            .json(payload)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(Self::classify_reqwest_error)?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body_snippet = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>();
            if body_snippet.is_empty() {
                anyhow::bail!("gateway /send returned {status}")
            } else {
                anyhow::bail!("gateway /send returned {status}: {body_snippet}")
            }
        }
    }

    /// Classify a reqwest transport error into a human-readable message.
    fn classify_reqwest_error(e: reqwest::Error) -> anyhow::Error {
        if e.is_connect() {
            anyhow::anyhow!("gateway unreachable (connection error): {e}")
        } else if e.is_timeout() {
            anyhow::anyhow!("gateway request timed out: {e}")
        } else {
            anyhow::anyhow!("gateway request failed: {e}")
        }
    }
}

#[async_trait]
impl MessageSender for GatewayMessageSender {
    async fn send(&self, text: &str) -> Result<SendOutcome> {
        let chat_id = self.resolve_chat_id().await?;

        // chat_id == 0 is the documented sentinel for "no reply channel" (GitHub
        // webhooks, non-Telegram sessions). Short-circuit before the HTTP POST —
        // no retry, no failed_sends entry, since this is a permanent condition.
        if chat_id == 0 {
            // Operator-visible: message content that was lost. Truncated to 500 chars
            // because the operator needs to assess severity of the lost escalation,
            // and most escalation messages fit within this budget.
            error!(
                agent_name = ?self.agent_name,
                request_id = ?self.request_id,
                message_text = %truncate_for_log(text, 500),
                "send_message_nochannel: message lost — chat_id=0, no reply channel available"
            );
            return Ok(SendOutcome::NoChannel);
        }

        tracing::debug!(
            agent_name = ?self.agent_name,
            chat_id,
            text_len = text.len(),
            "GatewayMessageSender: sending outbound message"
        );

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "request_id": self.request_id,
            "agent_name": self.agent_name,
        });

        // First attempt
        match self.try_send(&payload).await {
            Ok(()) => return Ok(SendOutcome::Delivered),
            Err(e) => warn!(error = %e, "first /send attempt failed, retrying in 2s"),
        }

        // Retry after 2s
        tokio::time::sleep(Duration::from_secs(2)).await;
        match self.try_send(&payload).await {
            Ok(()) => Ok(SendOutcome::Delivered),
            Err(e) => {
                warn!(error = %e, "retry failed, saving to failed_sends");
                let reason = e.to_string();
                // Capture the delivery failure reason before attempting DB write.
                // If save_failed_send fails, we still surface the original gateway
                // error to the caller (not the DB error).
                if let Err(db_err) = self.db.save_failed_send(text, None).await {
                    warn!(error = %db_err, "failed to save to failed_sends table");
                }
                Ok(SendOutcome::Failed { reason })
            }
        }
    }
}

/// A no-op message sender that silently accepts all messages.
///
/// Used to suppress user-facing notifications in contexts where the silent
/// turn should still run but not produce outbound messages (e.g., team-child
/// callback turns where the consolidated team notification handles delivery).
pub struct NoopSender;

#[async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Returns a UTF-8-safe prefix of `text` up to `max_chars` characters.
/// Uses `char_indices()` to find the byte boundary at the character limit,
/// avoiding panics on multi-byte UTF-8 when truncating.
fn truncate_for_log(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_resolve_chat_id_explicit_override() {
        // Delegate agent DB has no chat_id — but explicit override is set
        let harness = TestHarness::with_agent("delegate-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("delegate-agent".to_string()),
            Some(12345),
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(chat_id, 12345);
    }

    #[tokio::test]
    async fn test_resolve_chat_id_db_fallback() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "67890")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(chat_id, 67890);
    }

    #[tokio::test]
    async fn test_resolve_chat_id_no_override_no_db_fails() {
        // Neither explicit nor DB — should fail
        let harness = TestHarness::with_agent("delegate-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("delegate-agent".to_string()),
            None,
        );

        let err = sender.resolve_chat_id().await.unwrap_err();
        assert!(
            err.to_string().contains("chat_id not configured"),
            "unexpected error: {err}"
        );
    }

    /// Simulates the Telegram→GitHub→resolve sequence from #580:
    /// 1. Telegram message stores chat_id=99999
    /// 2. GitHub message with no chat_id must NOT overwrite it
    /// 3. resolve_chat_id returns the original 99999
    #[tokio::test]
    async fn test_github_message_does_not_poison_chat_id() {
        let harness = TestHarness::new();

        // Step 1: Telegram message stores the real chat_id
        harness
            .db
            .set_customer_config("chat_id", "99999")
            .await
            .unwrap();

        // Step 2: GitHub message arrives — handler would see chat_id=None and skip storage.
        // (The handler guard is `if let Some(chat_id) = req.chat_id && chat_id != 0`.)
        // We simulate by NOT writing to customer_config.

        // Step 3: resolve_chat_id should return the original Telegram chat_id
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(
            chat_id, 99999,
            "GitHub message must not overwrite Telegram chat_id"
        );
    }

    /// Verifies that a poisoned chat_id ("0") in the DB is parsed as 0.
    /// `resolve_chat_id()` returns the raw value — the `send()` method
    /// catches `chat_id == 0` and returns `SendOutcome::NoChannel` before
    /// the HTTP POST (#650).
    #[tokio::test]
    async fn test_resolve_chat_id_poisoned_zero_in_db() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "0")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None,
        );

        let chat_id = sender.resolve_chat_id().await.unwrap();
        assert_eq!(
            chat_id, 0,
            "poisoned value returns 0 — send() catches this as NoChannel"
        );
    }

    /// chat_id == 0 via explicit override returns NoChannel without HTTP POST (#650).
    #[tokio::test]
    async fn test_send_chat_id_zero_explicit_returns_no_channel() {
        let harness = TestHarness::with_agent("test-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("test-agent".to_string()),
            Some(0), // explicit chat_id override of 0
        );

        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(
            outcome,
            SendOutcome::NoChannel,
            "chat_id=0 should return NoChannel, not attempt HTTP POST"
        );
    }

    /// chat_id == 0 from DB (poisoned value) returns NoChannel via send() (#650).
    #[tokio::test]
    async fn test_send_chat_id_zero_from_db_returns_no_channel() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("chat_id", "0")
            .await
            .unwrap();

        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("mika".to_string()),
            None, // no explicit override — falls back to DB
        );

        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(
            outcome,
            SendOutcome::NoChannel,
            "poisoned chat_id=0 from DB should return NoChannel"
        );
    }

    #[tokio::test]
    async fn test_noop_sender_returns_delivered() {
        let sender = NoopSender;
        let outcome = sender.send("hello").await.unwrap();
        assert_eq!(outcome, SendOutcome::Delivered);
    }

    /// resolve_chat_id() returning Err (no chat_id configured) still propagates
    /// as Err, not NoChannel.
    #[tokio::test]
    async fn test_send_no_chat_id_configured_returns_err() {
        let harness = TestHarness::with_agent("no-chat-agent");
        let sender = GatewayMessageSender::new(
            "http://localhost:9999".to_string(),
            SecretString::from("test-token"),
            harness.db.clone(),
            reqwest::Client::new(),
            None,
            Some("no-chat-agent".to_string()),
            None, // no explicit override, no DB entry
        );

        let result = sender.send("hello").await;
        assert!(
            result.is_err(),
            "missing chat_id should return Err, not NoChannel"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("chat_id not configured"),
            "error should mention chat_id not configured"
        );
    }

    // --- truncate_for_log tests (mika#1090) ---

    #[test]
    fn test_truncate_for_log_empty() {
        assert_eq!(truncate_for_log("", 500), "");
    }

    #[test]
    fn test_truncate_for_log_shorter_than_limit() {
        assert_eq!(truncate_for_log("hello", 500), "hello");
    }

    #[test]
    fn test_truncate_for_log_exactly_at_limit() {
        let text = "a".repeat(500);
        assert_eq!(truncate_for_log(&text, 500), text.as_str());
    }

    #[test]
    fn test_truncate_for_log_longer_than_limit() {
        let text = "a".repeat(600);
        let result = truncate_for_log(&text, 500);
        assert_eq!(result.len(), 500);
        assert_eq!(result, &text[..500]);
    }

    #[test]
    fn test_truncate_for_log_multibyte_utf8() {
        // Each emoji is 4 bytes. 3 emojis = 12 bytes, 3 chars.
        let text = "🔥🔥🔥abc";
        // Truncate at 3 chars — should get exactly the 3 emojis (12 bytes)
        let result = truncate_for_log(text, 3);
        assert_eq!(result, "🔥🔥🔥");
        // Truncate at 5 chars — should get 3 emojis + "ab"
        let result = truncate_for_log(text, 5);
        assert_eq!(result, "🔥🔥🔥ab");
    }

    #[test]
    fn test_truncate_for_log_cjk_characters() {
        // CJK characters are 3 bytes each
        let text = "你好世界abcd";
        // Truncate at 4 chars
        let result = truncate_for_log(text, 4);
        assert_eq!(result, "你好世界");
    }

    #[test]
    fn test_truncate_for_log_zero_limit() {
        assert_eq!(truncate_for_log("hello", 0), "");
    }
}
