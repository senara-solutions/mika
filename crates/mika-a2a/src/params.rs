use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::{Message, Task, TaskPushNotificationConfig};

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

/// Request-metadata key naming the model this turn must run under (mika#2304).
///
/// Third of the `mika.*` family, and the **only one whose application is
/// fail-closed**. The value is the raw string the operator typed (`sonnet`,
/// `moonshotai/kimi-k2.5`) — alias resolution and prefix stripping happen on the
/// server, because they depend on the *executing* agent's `llm_provider`, which
/// the caller does not know on the `--remote` path (mika#1591 semantics).
///
/// **Reading the key is fail-soft, applying it is not.** Absent, `null`, a
/// non-string, or a blank string all mean "no override" — an older or newer
/// caller must not be able to fail a turn with a field this server is free to
/// ignore, exactly as for [`ONLY_SKILLS_KEY`]. But once an override is
/// *declared*, a server that cannot honour it **fails the turn** rather than
/// quietly running under its configured model. The asymmetry with its two
/// sisters is deliberate and measured by the ticket that added this key: a skill
/// restriction silently dropped makes a turn *wider*, which is visible and
/// falsifies no measurement; a model silently dropped makes the measurement
/// **wrong while producing a plausible answer** — "on croit tester un modèle et
/// on en teste un autre". A silent no-op here *is* the defect.
///
/// The spelling is the wire contract between `mika-cli` and `mika-agent`, which
/// share no dependency edge of their own — it lives here, in the crate that owns
/// [`MessageSendParams`], so neither side can rename it alone.
pub const MODEL_OVERRIDE_KEY: &str = "mika.model_override";

/// Response-metadata key carrying the model that **actually served** the turn
/// (mika#2304).
///
/// Written by the server on [`Task::metadata`] for every turn it runs, override
/// or not, in `provider/model` form. Read by `mika ask --verbose` instead of the
/// caller's own `Settings`.
///
/// **Why the attestation exists at all.** Before mika#2304 the CLI printed
/// `model:` from the local `Settings` it had just mutated with `--model` — so it
/// reported the requested model with full authority while the turn ran, at
/// mika-spirit, under the `config.toml` one. The field was not absent or null:
/// it asserted the override that had not happened. Propagating the override
/// without attesting it would only replace that false statement with an act of
/// faith, which is the same defect one step removed.
///
/// **Absence means "this server did not attest", never "the local value is
/// right".** A pre-mika#2304 server and a remote agent on another version both
/// land here; that is exactly the population where printing a local value would
/// be a lie.
pub const EFFECTIVE_MODEL_KEY: &str = "mika.effective_model";

/// Read the model a server attested for a finished [`Task`] (mika#2304).
///
/// Lives here rather than in either client surface so `mika ask`'s envelope and
/// `--remote`'s `--verbose` trailer read the same field the same way. Every
/// malformed shape — no metadata, key absent, `null`, non-string, blank — reads
/// as "not attested", which the caller must render as *absence of a model*, not
/// as a fallback to anything it knows locally.
pub fn attested_model(task: &Task) -> Option<&str> {
    task.metadata
        .as_ref()?
        .get(EFFECTIVE_MODEL_KEY)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

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

    #[test]
    fn model_override_key_is_the_wire_spelling() {
        // mika#2304. Third key of the family, same contract as its two sisters.
        assert_eq!(MODEL_OVERRIDE_KEY, "mika.model_override");
    }

    #[test]
    fn effective_model_key_is_the_wire_spelling() {
        // mika#2304. This one travels the other way — server to client — but is
        // no less a published contract for it.
        assert_eq!(EFFECTIVE_MODEL_KEY, "mika.effective_model");
    }

    #[test]
    fn the_three_request_keys_are_distinct() {
        // A copy-paste that collapsed two of them would make one flag silently
        // carry another's payload, and every per-key test would still pass.
        assert_ne!(CALLER_SESSION_ID_KEY, ONLY_SKILLS_KEY);
        assert_ne!(CALLER_SESSION_ID_KEY, MODEL_OVERRIDE_KEY);
        assert_ne!(ONLY_SKILLS_KEY, MODEL_OVERRIDE_KEY);
        assert_ne!(MODEL_OVERRIDE_KEY, EFFECTIVE_MODEL_KEY);
    }

    fn task_with_metadata(metadata: Option<serde_json::Value>) -> Task {
        let mut task = Task {
            id: "t-1".to_string(),
            context_id: None,
            status: crate::types::TaskStatus {
                state: crate::types::TaskState::Completed,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        };
        if let Some(serde_json::Value::Object(map)) = metadata {
            task.metadata = Some(map.into_iter().collect());
        }
        task
    }

    /// mika#2304 T6 half: a Task the server attested reads back verbatim.
    #[test]
    fn attested_model_reads_the_servers_value() {
        let task = task_with_metadata(Some(serde_json::json!({
            EFFECTIVE_MODEL_KEY: "openrouter/moonshotai/kimi-k2.5",
        })));
        assert_eq!(
            attested_model(&task),
            Some("openrouter/moonshotai/kimi-k2.5")
        );
    }

    /// mika#2304: every unreadable shape is "not attested", never a fallback.
    /// The caller must show nothing — showing a local value is the very lie the
    /// attestation exists to stop.
    #[test]
    fn every_unreadable_shape_reads_as_not_attested() {
        assert_eq!(attested_model(&task_with_metadata(None)), None);
        for shape in [
            serde_json::json!({}),
            serde_json::json!({ EFFECTIVE_MODEL_KEY: serde_json::Value::Null }),
            serde_json::json!({ EFFECTIVE_MODEL_KEY: 42 }),
            serde_json::json!({ EFFECTIVE_MODEL_KEY: ["a"] }),
            serde_json::json!({ EFFECTIVE_MODEL_KEY: "" }),
            serde_json::json!({ EFFECTIVE_MODEL_KEY: "   " }),
        ] {
            assert_eq!(
                attested_model(&task_with_metadata(Some(shape.clone()))),
                None,
                "shape {shape} should read as not attested"
            );
        }
    }
}
