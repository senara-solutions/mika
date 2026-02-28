use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Image payload forwarded from the gateway.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImagePayload {
    pub media_type: String,
    pub data: String,
}

/// Inbound message from the gateway.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MessageRequest {
    pub text: String,
    pub chat_id: i64,
    pub channel: String,
    pub request_id: String,
    /// Target agent name (defaults to the server's default agent if absent).
    #[serde(default)]
    pub agent: String,
    /// Optional images forwarded from the gateway (base64-encoded).
    #[serde(default)]
    pub images: Option<Vec<ImagePayload>>,
}

/// Accepted response for async processing.
#[derive(Debug, Serialize, ToSchema)]
pub struct AcceptedResponse {
    pub request_id: String,
    pub status: String,
}

/// Inbound heartbeat trigger from the gateway/K8s CronJob.
#[derive(Debug, Deserialize, ToSchema)]
pub struct HeartbeatRequest {
    pub request_id: String,
    /// Target agent name (defaults to the server's default agent if absent).
    #[serde(default)]
    pub agent: String,
}

/// Health check response body.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
}
