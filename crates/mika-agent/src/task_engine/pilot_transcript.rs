//! The pilot-transcript line contract (mika#1705, mika#2040).
//!
//! One JSONL line per claude-pilot SDK message, written by claude-pilot into
//! `$ANTHROPIC_LOG_FILE`, read here. This module is the **sole source of truth
//! for the line format**: the writer lives in another repository, so the format
//! has to be owned somewhere a reviewer can find it, and the reader is the only
//! side that can refuse a line.
//!
//! # Why the version field is mandatory, and why an unknown one is refused
//!
//! mika#1705 shipped the producer (`skills/executor.rs` injects
//! `ANTHROPIC_LOG_FILE`, `dispatch-lib.sh` mounts the directory) and this
//! ingestion, and never shipped the writer — for 25+ days the directory stayed
//! empty and nothing said so, because "no file" is indistinguishable from "no
//! session". The lesson mika#2040 draws is not only "write the writer": it is
//! that a cross-repo format with no version and no refusal fails silently in
//! *both* directions. A line whose `schema_version` is absent or unknown is
//! therefore refused **by name**, never ingested on a best-effort read of the
//! fields that happen to be recognisable.
//!
//! # v1 line
//!
//! ```json
//! {"schema_version":"v1","timestamp":"2026-09-07T12:00:00Z","provider":"anthropic",
//!  "model":"claude-sonnet-4-6","request_body":"…","response_body":"…",
//!  "tokens_in":1234,"tokens_out":56,"latency_ms":789}
//! ```
//!
//! `schema_version` is the only required field. Every other field is optional
//! and maps to a nullable column of `pilot_transcripts`: a message the SDK
//! reports without usage numbers is still worth keeping, and a transcript that
//! refused half a session's lines to enforce a field nobody needs would defeat
//! its own purpose. `request_body`/`response_body` may be a JSON string or a
//! JSON object/array — the object form is serialised to its compact string form
//! before scrubbing, because claude-pilot has both shapes available and forcing
//! a stringify on the writer buys nothing.
//!
//! Full spec, including the writer's obligations:
//! `docs/architecture/pilot-transcript-schema.md`.

use crate::db::PilotTranscriptRow;

/// The one schema version this build ingests.
///
/// Bumping it is a cross-repo change: a mika that only accepts `v2` refuses
/// every line a `v1` claude-pilot writes, and the refusal is loud but total.
/// Accept the new version alongside the old one for one release before
/// dropping the old one.
pub const PILOT_TRANSCRIPT_SCHEMA_VERSION: &str = "v1";

/// The JSON key carrying [`PILOT_TRANSCRIPT_SCHEMA_VERSION`].
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// Why one transcript line was refused (mika#2040 AC2).
///
/// Carries the offending value so the operator-facing message names what was
/// received rather than only what was expected — a refusal that does not say
/// what it saw sends the reader back to the file to find out.
#[derive(Debug, PartialEq, Eq)]
pub enum TranscriptLineError {
    /// No `schema_version` key at all. The pre-mika#2040 shape, and the shape
    /// of anything that writes to this path without knowing the contract.
    MissingVersion,
    /// A `schema_version` this build does not ingest.
    UnknownVersion(String),
    /// `schema_version` present but not a JSON string (e.g. `1` or `["v1"]`).
    MalformedVersion(String),
    /// The line is not a JSON object at all (e.g. a bare array or scalar).
    NotAnObject,
}

impl std::fmt::Display for TranscriptLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingVersion => write!(
                f,
                "missing `{SCHEMA_VERSION_KEY}` (expected \"{PILOT_TRANSCRIPT_SCHEMA_VERSION}\")"
            ),
            Self::UnknownVersion(v) => write!(
                f,
                "unknown `{SCHEMA_VERSION_KEY}` \"{v}\" \
                 (this build ingests \"{PILOT_TRANSCRIPT_SCHEMA_VERSION}\" only)"
            ),
            Self::MalformedVersion(v) => write!(
                f,
                "malformed `{SCHEMA_VERSION_KEY}` {v} — expected the JSON string \
                 \"{PILOT_TRANSCRIPT_SCHEMA_VERSION}\""
            ),
            Self::NotAnObject => write!(f, "line is not a JSON object"),
        }
    }
}

/// Parse one claude-pilot transcript JSONL object into a [`PilotTranscriptRow`].
///
/// Refuses the line when the schema version is absent, malformed, or unknown
/// (mika#2040 AC2). Every other field is optional: missing ones become `None`,
/// and body fields are secret-scrubbed. Body values that are JSON
/// objects/arrays are serialised to their compact string form before scrubbing.
pub fn parse_pilot_transcript_line(
    v: &serde_json::Value,
) -> Result<PilotTranscriptRow, TranscriptLineError> {
    let serde_json::Value::Object(_) = v else {
        return Err(TranscriptLineError::NotAnObject);
    };

    match v.get(SCHEMA_VERSION_KEY) {
        None | Some(serde_json::Value::Null) => return Err(TranscriptLineError::MissingVersion),
        Some(serde_json::Value::String(s)) if s == PILOT_TRANSCRIPT_SCHEMA_VERSION => {}
        Some(serde_json::Value::String(s)) => {
            return Err(TranscriptLineError::UnknownVersion(s.clone()));
        }
        Some(other) => return Err(TranscriptLineError::MalformedVersion(other.to_string())),
    }

    let str_field = |key: &str| v.get(key).and_then(|x| x.as_str()).map(str::to_owned);
    let i64_field = |key: &str| v.get(key).and_then(serde_json::Value::as_i64);
    let scrubbed_body = |key: &str| -> Option<String> {
        match v.get(key) {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => {
                Some(crate::secret_scrubber::scrub_secrets(s).into_owned())
            }
            Some(other) => {
                Some(crate::secret_scrubber::scrub_secrets(&other.to_string()).into_owned())
            }
        }
    };

    Ok(PilotTranscriptRow {
        timestamp: str_field("timestamp"),
        provider: str_field("provider"),
        model: str_field("model"),
        request_body: scrubbed_body("request_body"),
        response_body: scrubbed_body("response_body"),
        tokens_in: i64_field("tokens_in"),
        tokens_out: i64_field("tokens_out"),
        latency_ms: i64_field("latency_ms"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_line_parses_every_field() {
        let row = parse_pilot_transcript_line(&json!({
            "schema_version": "v1",
            "timestamp": "2026-09-07T12:00:00Z",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "request_body": "hello",
            "response_body": "world",
            "tokens_in": 12,
            "tokens_out": 34,
            "latency_ms": 56,
        }))
        .expect("v1 line must parse");

        assert_eq!(row.timestamp.as_deref(), Some("2026-09-07T12:00:00Z"));
        assert_eq!(row.provider.as_deref(), Some("anthropic"));
        assert_eq!(row.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(row.request_body.as_deref(), Some("hello"));
        assert_eq!(row.response_body.as_deref(), Some("world"));
        assert_eq!(row.tokens_in, Some(12));
        assert_eq!(row.tokens_out, Some(34));
        assert_eq!(row.latency_ms, Some(56));
    }

    #[test]
    fn only_the_version_is_required() {
        let row = parse_pilot_transcript_line(&json!({ "schema_version": "v1" }))
            .expect("a version-only line is a valid, if empty, v1 line");
        assert_eq!(row.timestamp, None);
        assert_eq!(row.tokens_in, None);
    }

    #[test]
    fn object_bodies_are_serialized_not_dropped() {
        let row = parse_pilot_transcript_line(&json!({
            "schema_version": "v1",
            "response_body": {"type": "text", "text": "hi"},
        }))
        .expect("object body is accepted");
        let body = row.response_body.expect("object body must be kept");
        assert!(body.contains("\"text\""), "got {body}");
    }

    #[test]
    fn a_line_without_a_version_is_refused() {
        // The pre-mika#2040 shape. Refusing it is the whole point of AC2:
        // ingesting it would make an unversioned producer look supported.
        let err = parse_pilot_transcript_line(&json!({
            "timestamp": "2026-09-07T12:00:00Z",
            "model": "claude-sonnet-4-6",
        }))
        .expect_err("an unversioned line must be refused");
        assert_eq!(err, TranscriptLineError::MissingVersion);
    }

    #[test]
    fn an_unknown_version_is_refused_and_named() {
        let err = parse_pilot_transcript_line(&json!({"schema_version": "v2"}))
            .expect_err("an unknown version must be refused");
        assert_eq!(err, TranscriptLineError::UnknownVersion("v2".to_string()));
        // The message must carry the received value — a refusal that only
        // states the expectation sends the reader back to the file.
        assert!(err.to_string().contains("\"v2\""), "got {err}");
    }

    #[test]
    fn a_non_string_version_is_refused() {
        let err = parse_pilot_transcript_line(&json!({"schema_version": 1}))
            .expect_err("a numeric version must be refused");
        assert_eq!(err, TranscriptLineError::MalformedVersion("1".to_string()));
    }

    #[test]
    fn a_non_object_line_is_refused() {
        let err = parse_pilot_transcript_line(&json!(["v1"]))
            .expect_err("a JSON array is not a transcript line");
        assert_eq!(err, TranscriptLineError::NotAnObject);
    }

    #[test]
    fn secrets_in_bodies_are_scrubbed_before_they_reach_the_row() {
        let row = parse_pilot_transcript_line(&json!({
            "schema_version": "v1",
            "request_body": "export GH_TOKEN=ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        }))
        .expect("v1 line must parse");
        let body = row.request_body.expect("body kept");
        assert!(
            !body.contains("ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            "secret survived scrubbing: {body}"
        );
    }
}
