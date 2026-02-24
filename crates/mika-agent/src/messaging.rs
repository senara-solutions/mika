use anyhow::Result;
use async_trait::async_trait;

/// Trait for sending outbound messages to the user.
/// CLI mode uses the fallback in send_message tool; HTTP mode will POST to the gateway.
#[async_trait(?Send)]
pub trait MessageSender {
    async fn send(&self, text: &str) -> Result<()>;
}
