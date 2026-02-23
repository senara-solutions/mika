use serde::{Deserialize, Serialize};

/// Message from routing layer to agent container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_type: String,
    pub channel_user_id: String,
    pub text: String,
    #[serde(default)]
    pub message_id: Option<String>,
}

/// Message from agent container to routing layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub customer_id: String,
    pub channel_type: String,
    pub channel_user_id: String,
    pub text: String,
}

/// Typing indicator request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingRequest {
    pub customer_id: String,
    pub channel_type: String,
    pub channel_user_id: String,
}
