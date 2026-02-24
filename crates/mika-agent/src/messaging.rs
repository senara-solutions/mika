use anyhow::Result;
use async_trait::async_trait;

/// Trait for sending outbound messages to the user.
/// CLI mode logs to stdout; HTTP mode POSTs to the gateway.
#[async_trait(?Send)]
pub trait MessageSender {
    async fn send(&self, text: &str) -> Result<()>;
}

/// CLI implementation: just logs the message.
pub struct CliMessageSender;

#[async_trait(?Send)]
impl MessageSender for CliMessageSender {
    async fn send(&self, text: &str) -> Result<()> {
        tracing::info!(text, "send_message (CLI mode)");
        Ok(())
    }
}
