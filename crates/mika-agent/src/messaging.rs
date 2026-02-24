use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::time::Duration;
use tracing::warn;

use crate::async_db::AsyncDatabase;

/// Trait for sending outbound messages to the user.
/// CLI mode uses the fallback in send_message tool; HTTP mode will POST to the gateway.
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
    internal_token: String,
    db: AsyncDatabase,
}

impl GatewayMessageSender {
    pub fn new(
        gateway_url: String,
        internal_token: String,
        db: AsyncDatabase,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            gateway_url,
            internal_token,
            db,
        }
    }

    async fn try_send(&self, payload: &serde_json::Value) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/send", self.gateway_url))
            .bearer_auth(&self.internal_token)
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

        let payload = serde_json::json!({ "chat_id": chat_id, "text": text });

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
