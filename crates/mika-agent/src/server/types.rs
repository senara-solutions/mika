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

/// Health check response body.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
}

/// Request body for POST /tasks/{id}/complete.
///
/// Marks a `callback` trigger task as completed with an optional result payload,
/// then triggers `resume_agent` dispatch if the task's action_type is `resume_agent`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TaskCompleteRequest {
    /// Result payload from the background process (required for resume_agent tasks).
    pub result: String,
    /// Target agent name (defaults to the server's default agent if absent or empty).
    #[serde(default)]
    pub agent: String,
}

/// Response body for POST /tasks/{id}/complete.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskCompleteResponse {
    pub task_id: String,
    pub status: String,
}

/// Request body for POST /tasks/{id}/cancel.
///
/// Cancels a task and kills its running process (if any).
#[derive(Debug, Deserialize, ToSchema)]
pub struct TaskCancelRequest {
    /// Target agent name (defaults to the server's default agent if absent or empty).
    #[serde(default)]
    pub agent: String,
}

/// Response body for POST /tasks/{id}/cancel.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskCancelResponse {
    pub task_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_killed: Option<bool>,
}
