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

/// Request-metadata key naming the ONLY skills this turn should carry (mika#2363).
///
/// A `message/send` caller that knows which pass it is running may name it. The
/// server then restricts the turn's skill registry to the named set — **by
/// subtraction only**: every skill whose name is absent from the list is
/// transiently disabled, and a named skill that would not otherwise have been
/// active is *not* resurrected. The field can therefore never widen a turn's
/// surface, which is what makes it safe on an endpoint any authenticated caller
/// can reach.
///
/// The founding measurement: mika-arch declares three `always_on` skills
/// (`mika-arch-groom-ticket`, `mika-arch-second-review`,
/// `mika-arch-groom-milestone`, 39 798 bytes of prompt in total) and every turn
/// runs exactly **one** of those passes — so 23.5 KB of every architect system
/// prompt described two tasks the turn was not doing.
///
/// Advisory in both directions: a server free to ignore the key, a caller free
/// to omit it. Absent, empty, or not an array of strings all mean "no
/// restriction", so an older caller and a newer one produce the same turn.
///
/// The spelling is the wire contract between `mika-cli` and `mika-agent`, which
/// share no dependency edge of their own — it lives here, in the crate that owns
/// [`MessageSendParams`], so neither side can rename it alone.
pub const ONLY_SKILLS_KEY: &str = "mika.only_skills";

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

    #[test]
    fn only_skills_key_is_the_wire_spelling() {
        // mika#2363. Same contract as above: `mika-cli` writes this key and
        // `mika-agent` reads it, with no dependency edge between them that a
        // rename could travel along.
        assert_eq!(ONLY_SKILLS_KEY, "mika.only_skills");
    }
}
