use anyhow::{Result, anyhow};
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;
use tracing::warn;

use crate::async_db::AsyncDatabase;

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
    async fn send(&self, text: &str) -> Result<()>;
}

/// Sends outbound messages by POSTing to the gateway's /send endpoint.
///
/// On failure, retries once after 2s. If both attempts fail, saves to
/// `failed_sends` table for later flush — returns Ok so the agent loop
/// doesn't think the tool failed.
pub struct GatewayMessageSender {
    client: reqwest::Client,
    gateway_url: String,
    internal_token: SecretString,
    db: AsyncDatabase,
    request_id: Option<String>,
    agent_name: Option<String>,
}

impl GatewayMessageSender {
    pub fn new(
        gateway_url: String,
        internal_token: SecretString,
        db: AsyncDatabase,
        client: reqwest::Client,
        request_id: Option<String>,
        agent_name: Option<String>,
    ) -> Self {
        Self {
            client,
            gateway_url,
            internal_token,
            db,
            request_id,
            agent_name,
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
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("gateway /send returned {}", resp.status())
        }
    }
}

#[async_trait]
impl MessageSender for GatewayMessageSender {
    async fn send(&self, text: &str) -> Result<()> {
        let chat_id = self
            .db
            .get_customer_config("chat_id")
            .await?
            .ok_or_else(|| anyhow!("chat_id not configured — no Telegram pairing yet"))?
            .parse::<i64>()
            .map_err(|e| anyhow!("invalid chat_id: {e}"))?;

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "request_id": self.request_id,
            "agent_name": self.agent_name,
        });

        // First attempt
        match self.try_send(&payload).await {
            Ok(()) => return Ok(()),
            Err(e) => warn!(error = %e, "first /send attempt failed, retrying in 2s"),
        }

        // Retry after 2s
        tokio::time::sleep(Duration::from_secs(2)).await;
        match self.try_send(&payload).await {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(error = %e, "retry failed, saving to failed_sends");
                self.db.save_failed_send(text, None).await?;
                Ok(()) // Return Ok — message queued, don't confuse Claude
            }
        }
    }
}
