use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::{Message, TaskPushNotificationConfig};

/// Request-metadata key carrying the caller's own session id (mika#2070).
///
/// A `message/send` caller may name the session it is already keeping locally.
/// A server that shares the caller's database can then run the turn under that
/// session, so the `turn_usage` it logs is attributable to the caller's run
/// instead of to a server-minted id. Advisory in both directions: a server free
/// to ignore the key, a caller free to omit it.
///
/// The spelling is the wire contract between `mika-cli` and `mika-agent`, which
/// share no dependency edge of their own — it lives here, in the crate that owns
/// [`MessageSendParams`], so neither side can rename it alone.
pub const CALLER_SESSION_ID_KEY: &str = "mika.caller_session_id";

/// Parameters for `message/send` and `message/stream`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Configuration for sending a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_output_modes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_push_notification_config: Option<TaskPushNotificationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_immediately: Option<bool>,
}

/// Parameters for `tasks/get`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueryParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_length: Option<i32>,
}

/// Parameters for `tasks/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_session_id_key_is_the_wire_spelling() {
        // External A2A clients may send this key, so its spelling is a published
        // contract, not an internal detail. Changing it is a protocol change.
        assert_eq!(CALLER_SESSION_ID_KEY, "mika.caller_session_id");
    }
}
