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

/// Request-metadata key asking for this turn to read its own session only
/// (mika#1951).
///
/// Fourth of the `mika.*` family. `true` ⇒ the turn's conversation window is
/// restricted to the caller's session **and** no compaction summary is injected,
/// whatever the agent's `identity.toml` says. Absent or `null` ⇒ no restriction,
/// and the turn is the one before this key existed, bit for bit. `false` is
/// honoured literally and is the same no-op.
///
/// # A bool, never a scope name — and that is a safety constraint
///
/// `/a2a/{agent}` is reachable by any authenticated caller. A key carrying a
/// scope *name* (`"agent"` / `"session"`) would let a caller ask for `"agent"` on an agent
/// configured `"session"`, i.e. **widen** mika-arch's window from the network and
/// make it read other tickets' plans — reopening mika#2295 and mika#2305 through
/// the door. A bool whose only effective value is `true` makes widening
/// *inexpressible by construction* rather than refused by a predicate a later
/// editor could relax. Same reasoning as [`ONLY_SKILLS_KEY`]'s strictly
/// subtractive contract.
///
/// # Reading is fail-soft on absence, fail-closed on a malformed value
///
/// Absent and `null` mean "no restriction", so an older or a newer caller
/// produces the same turn — the property the three sisters already have. But a
/// key that is *present with a non-boolean value* **fails the request**; it is
/// never read as `false`. The asymmetry with [`ONLY_SKILLS_KEY`] is the same one
/// [`MODEL_OVERRIDE_KEY`] carries, for the same measured reason: a skill
/// restriction silently dropped makes a turn *wider*, which is visible and
/// falsifies no measurement, while an **isolation** silently dropped makes the
/// measurement wrong while producing a plausible answer. That is literally the
/// defect of mika#1951 — the 2026-08-22 battery produced contaminated data that
/// looked valid. A silent no-op here *is* the defect.
///
/// The spelling is the wire contract between `mika-cli` and `mika-agent`, which
/// share no dependency edge of their own — it lives here, in the crate that owns
/// [`MessageSendParams`], so neither side can rename it alone.
pub const SESSION_ISOLATED_KEY: &str = "mika.session_isolated";

/// Response-metadata key carrying whether the turn **really ran** isolated
/// (mika#1951).
///
/// Written by the server on [`Task::metadata`] for every synchronous
/// `message/send` turn, isolated or not. Read by `mika ask --verbose` instead of
/// the flag the caller passed.
///
/// **Why the attestation is the half that counts.** Without it, a bench run
/// against a mika-spirit predating this key would have `--isolated` accepted by
/// its CLI, ignored by the server, and displayed as isolated with full
/// authority. That is exactly the false green mika#2304 measured — `model:`
/// printing the requested model while the turn ran under the `config.toml` one —
/// and it is exactly the defect of mika#1951: believing you measure isolated and
/// not being. Shipping the request key without this one would replace a
/// contamination with an act of faith.
///
/// **Absence means "this server did not attest", never "the local flag was
/// honoured".** A pre-mika#1951 server, a remote agent on another version, and
/// every path outside synchronous `message/send` all land there.
pub const SESSION_ISOLATED_APPLIED_KEY: &str = "mika.session_isolated_applied";

/// Response-metadata key carrying the token usage of the **whole turn**
/// (mika#1883).
///
/// Written by the server on [`Task::metadata`] when the turn produced at least
/// one LLM call whose usage it could read. Read by `mika ask --verbose` and by
/// `mika ask --remote --verbose`, which render it as `tokens.*`.
///
/// # What "the turn" means, exactly
///
/// The **sum** of every LLM call this turn made — the step loop plus the
/// max-steps continuation call — not the last one. That distinction is the
/// whole reason the field exists: `AgentOutput.usage` carries the usage of the
/// *last* call (the loop overwrites it at each step), so a turn that spends its
/// twenty tool steps makes twenty-one calls and would report the twenty-first
/// under the label "per-run usage" — a plausible number, presented with
/// authority, that undercounts massively. That is mika#2304's defect transposed
/// one field over.
///
/// It does **not** include what the turn fanned out: a `delegate_task` or a team
/// run spends under `delegate-*` / `team-*` sessions of its own. This is the
/// turn, never the campaign — the same bound `mika ask` already documents for
/// [`CALLER_SESSION_ID_KEY`].
///
/// # RAW, never normalised
///
/// `input` is whatever the provider reported. Anthropic reports fresh input;
/// the OpenAI-compatible rails report `prompt_tokens`, which **includes**
/// `cache_read`. Normalising server-side would create a second truth diverging
/// from the `turn_usage` log stream, which is RAW by an explicit decision
/// (mika#1889). Read `mika.effective_model` from the same Task before comparing
/// two numbers across rails.
///
/// # Absence, never a zero
///
/// The key is **omitted** when the turn produced no usage to read. A rendered
/// `0` would be indistinguishable from a real turn and would assert a
/// measurement that did not happen — the same reasoning `llm_calls`'
/// `request_bytes` carries (mika#2331: *"`null` is never `0`"*). Unlike
/// [`EFFECTIVE_MODEL_KEY`] and [`SESSION_ISOLATED_APPLIED_KEY`], it is therefore
/// **not** written unconditionally: those two are the answer to a flag, where an
/// absence would be ambiguous between "this server is old" and "nothing was
/// asked for", while this one is a measurement whose only legitimate absence is
/// "there was nothing to measure". The residual ambiguity — a pre-mika#1883
/// server and an unmeasured turn read alike — is accepted and named: both call
/// for the same client conduct, showing nothing.
///
/// The spelling is the wire contract between `mika-cli` and `mika-agent`, which
/// share no dependency edge of their own — it lives here, in the crate that owns
/// [`MessageSendParams`], so neither side can rename it alone.
pub const RUN_USAGE_KEY: &str = "mika.run_usage";

/// Token usage of one whole turn, as it travels on [`RUN_USAGE_KEY`].
///
/// One object rather than four flat `mika.*` keys: the client renders the four
/// numbers as a group (`tokens.input`, `tokens.output`, …), and a family of four
/// keys for a single measurement would crowd a namespace shared by five
/// unrelated features.
///
/// The two cache fields are absent when the provider reported none — see
/// [`RUN_USAGE_KEY`] on why absence is never encoded as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub input: u64,
    pub output: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
}

/// Read the per-turn usage a server attested for a finished [`Task`]
/// (mika#1883).
///
/// The third sibling of [`attested_model`] and [`attested_session_isolation`],
/// and it lives beside them for the reason their doc comments give: both client
/// surfaces must read the same field the same way, or `mika ask --verbose` and
/// `mika ask --remote --verbose` answer the same question differently.
///
/// Every malformed shape — no metadata, key absent, `null`, not an object, a
/// missing or non-integer `input`/`output`, a negative number — reads as *not
/// attested*, which the caller must render as **absence of the `tokens.*`
/// lines**, never as a zero. A cache field that is present but unreadable is
/// dropped on its own rather than discarding the whole reading: the two totals
/// are still true, and losing them to a malformed optional would be a worse
/// answer than losing the optional.
pub fn attested_run_usage(task: &Task) -> Option<RunUsage> {
    let value = task.metadata.as_ref()?.get(RUN_USAGE_KEY)?.as_object()?;
    Some(RunUsage {
        input: value.get("input")?.as_u64()?,
        output: value.get("output")?.as_u64()?,
        cache_read: value.get("cache_read").and_then(serde_json::Value::as_u64),
        cache_write: value.get("cache_write").and_then(serde_json::Value::as_u64),
    })
}

/// Read the session isolation a server attested for a finished [`Task`]
/// (mika#1951).
///
/// The bool sibling of [`attested_model`], and it lives beside it for the same
/// reason: both client surfaces must read the same field the same way. Every
/// malformed shape — no metadata, key absent, `null`, a non-boolean — reads as
/// "not attested", which the caller must render as *absence*, never as a
/// fallback to the flag it passed.
pub fn attested_session_isolation(task: &Task) -> Option<bool> {
    task.metadata
        .as_ref()?
        .get(SESSION_ISOLATED_APPLIED_KEY)?
        .as_bool()
}

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
    fn session_isolated_keys_are_the_wire_spelling() {
        // mika#1951. Fourth request key and its response attestation, same
        // contract as the three before them.
        assert_eq!(SESSION_ISOLATED_KEY, "mika.session_isolated");
        assert_eq!(
            SESSION_ISOLATED_APPLIED_KEY,
            "mika.session_isolated_applied"
        );
    }

    #[test]
    fn run_usage_key_is_the_wire_spelling() {
        // mika#1883. Third response key of the family, same contract: the CLI
        // reads it and the agent writes it, with no dependency edge between
        // them for a rename to travel along.
        assert_eq!(RUN_USAGE_KEY, "mika.run_usage");
    }

    #[test]
    fn every_key_of_the_family_is_distinct() {
        // A copy-paste that collapsed two of them would make one flag silently
        // carry another's payload, and every per-key test would still pass.
        //
        // mika#1951 widened this from the hand-written pair list it used to be:
        // the request/response pair added here differ by one suffix, which is
        // exactly the shape a pairwise list forgets to cover.
        let keys = [
            CALLER_SESSION_ID_KEY,
            ONLY_SKILLS_KEY,
            MODEL_OVERRIDE_KEY,
            EFFECTIVE_MODEL_KEY,
            SESSION_ISOLATED_KEY,
            SESSION_ISOLATED_APPLIED_KEY,
            RUN_USAGE_KEY,
        ];
        let unique: std::collections::HashSet<&str> = keys.iter().copied().collect();
        assert_eq!(
            unique.len(),
            keys.len(),
            "two `mika.*` keys share a spelling: {keys:?}"
        );
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

    /// mika#1951 U3 — the attestation reads back both verdicts, and only a real
    /// boolean counts as one.
    ///
    /// `false` must be readable as an attested `false`: "this server ran the turn
    /// and it was NOT isolated" is a different statement from "this server said
    /// nothing", and the bench operator acts differently on each.
    #[test]
    fn attested_session_isolation_reads_both_verdicts() {
        for declared in [true, false] {
            let task = task_with_metadata(Some(serde_json::json!({
                SESSION_ISOLATED_APPLIED_KEY: declared,
            })));
            assert_eq!(attested_session_isolation(&task), Some(declared));
        }
    }

    /// mika#1883 — a Task the server attested reads back verbatim, and the two
    /// cache fields are optional independently of the two totals.
    #[test]
    fn attested_run_usage_reads_the_servers_value() {
        let task = task_with_metadata(Some(serde_json::json!({
            RUN_USAGE_KEY: {"input": 41_000, "output": 900, "cache_read": 38_000, "cache_write": 12},
        })));
        assert_eq!(
            attested_run_usage(&task),
            Some(RunUsage {
                input: 41_000,
                output: 900,
                cache_read: Some(38_000),
                cache_write: Some(12),
            })
        );

        // A provider that reports no cache at all still attests its two totals.
        let no_cache = task_with_metadata(Some(serde_json::json!({
            RUN_USAGE_KEY: {"input": 10, "output": 2},
        })));
        assert_eq!(
            attested_run_usage(&no_cache),
            Some(RunUsage {
                input: 10,
                output: 2,
                cache_read: None,
                cache_write: None,
            })
        );
    }

    /// mika#1883 — every unreadable shape is *not attested*, which the caller
    /// must render as no `tokens.*` line at all.
    ///
    /// A zero is in the list deliberately as a **positive** case, one assertion
    /// down: `{"input": 0, "output": 0}` is a server saying it measured zero,
    /// which is a different statement from a server saying nothing, and only
    /// the *absence* of the key may render as absence.
    #[test]
    fn every_unreadable_run_usage_shape_reads_as_not_attested() {
        assert_eq!(attested_run_usage(&task_with_metadata(None)), None);
        for shape in [
            serde_json::json!({}),
            serde_json::json!({ RUN_USAGE_KEY: serde_json::Value::Null }),
            serde_json::json!({ RUN_USAGE_KEY: 42 }),
            serde_json::json!({ RUN_USAGE_KEY: "41000" }),
            serde_json::json!({ RUN_USAGE_KEY: [1, 2] }),
            // Half an answer is not an answer: a total missing its sibling
            // cannot be rendered without inventing the other.
            serde_json::json!({ RUN_USAGE_KEY: {"input": 10} }),
            serde_json::json!({ RUN_USAGE_KEY: {"output": 10} }),
            // A negative or fractional count is not a token count.
            serde_json::json!({ RUN_USAGE_KEY: {"input": -1, "output": 2} }),
            serde_json::json!({ RUN_USAGE_KEY: {"input": 1.5, "output": 2} }),
        ] {
            assert_eq!(
                attested_run_usage(&task_with_metadata(Some(shape.clone()))),
                None,
                "shape {shape} should read as not attested"
            );
        }

        // A malformed *optional* drops on its own — the two totals are still
        // true, and losing them to a bad cache field would be the worse answer.
        let bad_cache = task_with_metadata(Some(serde_json::json!({
            RUN_USAGE_KEY: {"input": 10, "output": 2, "cache_read": "lots"},
        })));
        assert_eq!(
            attested_run_usage(&bad_cache),
            Some(RunUsage {
                input: 10,
                output: 2,
                cache_read: None,
                cache_write: None,
            })
        );
    }

    /// mika#1951 U3 — every unreadable shape is absence, never the caller's flag.
    ///
    /// `"true"` as a string is in the list deliberately: it is the shape a
    /// hand-written client is most likely to produce, and reading it as `true`
    /// would let the attestation confirm an isolation nothing performed.
    #[test]
    fn every_unreadable_isolation_shape_reads_as_not_attested() {
        assert_eq!(attested_session_isolation(&task_with_metadata(None)), None);
        for shape in [
            serde_json::json!({}),
            serde_json::json!({ SESSION_ISOLATED_APPLIED_KEY: serde_json::Value::Null }),
            serde_json::json!({ SESSION_ISOLATED_APPLIED_KEY: "true" }),
            serde_json::json!({ SESSION_ISOLATED_APPLIED_KEY: 1 }),
            serde_json::json!({ SESSION_ISOLATED_APPLIED_KEY: [true] }),
        ] {
            assert_eq!(
                attested_session_isolation(&task_with_metadata(Some(shape.clone()))),
                None,
                "shape {shape} should read as not attested"
            );
        }
    }
}
