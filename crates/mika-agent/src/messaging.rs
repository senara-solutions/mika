use anyhow::{Result, anyhow};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;
use tracing::warn;

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
}
