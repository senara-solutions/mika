use serde::{Deserialize, Serialize};

/// Inbound message from the gateway.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub text: String,
    pub chat_id: i64,
    pub channel: String,
    pub request_id: String,
}

/// Accepted response for async processing.
#[derive(Debug, Serialize)]
pub struct AcceptedResponse {
    pub request_id: String,
    pub status: String,
}

/// Inbound heartbeat trigger from the gateway/K8s CronJob.
#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub request_id: String,
}

/// Health check response body.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
}
