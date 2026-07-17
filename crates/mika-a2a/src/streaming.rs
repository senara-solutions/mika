use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::types::{Artifact, Message, Task, TaskStatus};

/// Shared broadcast sender for A2A SSE frames (mika#1731).
///
/// Used by `handle_message_stream` to publish frames to any subscribed
/// SSE reader, and by the tool-execution layer to inject `ToolCallStart` /
/// `ToolCallResult` frames from `process_tool_calls`. Cheap to clone —
/// `broadcast::Sender` is internally reference-counted.
pub type StreamEventSender = Arc<tokio::sync::broadcast::Sender<StreamEvent>>;

/// Server-Sent Event from an A2A streaming response.
///
/// Consumers MUST tolerate unknown `kind` values via the `Unknown` catch-all
/// variant — new variants may ship in future releases (mika#1731 adds
/// `tool-call-start` / `tool-call-result`; mika#1732 will add task-event
/// frames; mika#1734 will add AskUserQuestion frames). Producers append new
/// variants; consumers cross the version gap by matching `Unknown` and
/// falling through to a "future frame, ignore" branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEvent {
    #[serde(rename = "task")]
    Task(Task),
    #[serde(rename = "message")]
    Message(Message),
    #[serde(rename = "status-update")]
    StatusUpdate(TaskStatusUpdateEvent),
    #[serde(rename = "artifact-update")]
    ArtifactUpdate(TaskArtifactUpdateEvent),
    /// Tool-call dispatch begin (mika#1731). Emitted immediately before the
    /// engine calls `execute_tool()` in `process_tool_calls`. Consumers that
    /// render fine-grained "running Bash: X" receipts subscribe to this.
    #[serde(rename = "tool-call-start")]
    ToolCallStart(ToolCallStartEvent),
    /// Tool-call dispatch result (mika#1731). Emitted immediately after
    /// `execute_tool()` returns, before persistence. Carries a truncated
    /// output preview; consumers can fetch the full call via
    /// `GET /api/v1/traces/:trace_id/tool-calls`.
    #[serde(rename = "tool-call-result")]
    ToolCallResult(ToolCallResultEvent),
    /// Forward-compat catch-all for `kind` values shipped in a future release
    /// this consumer does not recognize. Payload is preserved as raw JSON so
    /// tooling can inspect it if needed. See enum-level docstring.
    #[serde(other)]
    Unknown,
}

/// A task status update event sent via SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    #[serde(rename = "final")]
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// An artifact update event sent via SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub artifact: Artifact,
    #[serde(default)]
    pub append: bool,
    #[serde(default)]
    pub last_chunk: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Tool-call dispatch begin (mika#1731). Payload for
/// `StreamEvent::ToolCallStart`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallStartEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Step index within the current turn (matches `ToolCallSummary.step`).
    pub step: u32,
    pub tool_name: String,
    /// Truncated JSON preview of the arguments (UTF-8 safe, capped at
    /// `TOOL_CALL_SUMMARY_CAP_CHARS`).
    pub args_summary: String,
    /// UTC timestamp of the emit call.
    pub timestamp: String,
}

/// Tool-call dispatch result (mika#1731). Payload for
/// `StreamEvent::ToolCallResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub step: u32,
    pub tool_name: String,
    pub success: bool,
    pub non_zero_exit: bool,
    /// Truncated preview of the tool output (UTF-8 safe, capped at
    /// `TOOL_CALL_SUMMARY_CAP_CHARS`). Full text remains queryable via
    /// `GET /api/v1/traces/:trace_id/tool-calls`.
    pub output_summary: String,
    /// Wall-clock duration of the tool dispatch in milliseconds.
    pub duration_ms: u64,
    pub timestamp: String,
}

/// Character cap for `args_summary` and `output_summary` on the tool-call
/// SSE frames (mika#1731). Frames are glanceable receipts — the TUI renders
/// them inline for real-time visibility; full call bodies remain accessible
/// via `GET /api/v1/traces/:trace_id/tool-calls` (dashboard 50 KB cap).
pub const TOOL_CALL_SUMMARY_CAP_CHARS: usize = 500;

/// UTF-8-safe truncation for tool-call preview fields. Cuts at a codepoint
/// boundary and appends `…` when truncation actually removed characters.
/// Avoids byte-slice panics on multi-byte UTF-8 (see mika#764 lint script).
pub fn truncate_tool_summary(s: &str) -> String {
    if s.chars().count() <= TOOL_CALL_SUMMARY_CAP_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(TOOL_CALL_SUMMARY_CAP_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Part, Role, TaskState};

    fn make_status_update() -> StreamEvent {
        StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "task-1".to_string(),
            context_id: Some("ctx-1".to_string()),
            status: TaskStatus {
                state: TaskState::Working,
                message: None,
                timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            },
            is_final: false,
            metadata: None,
        })
    }

    fn make_artifact_update() -> StreamEvent {
        StreamEvent::ArtifactUpdate(TaskArtifactUpdateEvent {
            task_id: "task-1".to_string(),
            context_id: None,
            artifact: Artifact {
                artifact_id: "art-1".to_string(),
                name: Some("output.txt".to_string()),
                description: None,
                parts: vec![Part::Text {
                    text: "result".to_string(),
                    metadata: None,
                }],
                metadata: None,
                extensions: None,
            },
            append: false,
            last_chunk: true,
            metadata: None,
        })
    }

    fn make_task_event() -> StreamEvent {
        StreamEvent::Task(Task {
            id: "task-1".to_string(),
            context_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        })
    }

    fn make_message_event() -> StreamEvent {
        StreamEvent::Message(Message {
            message_id: "msg-1".to_string(),
            role: Role::Agent,
            parts: vec![Part::Text {
                text: "response".to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: Some("task-1".to_string()),
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        })
    }

    #[test]
    fn status_update_round_trip() {
        let event = make_status_update();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["kind"], "status-update");
        if let StreamEvent::StatusUpdate(ref e) = parsed {
            assert_eq!(e.task_id, "task-1");
            assert_eq!(e.status.state, TaskState::Working);
            assert!(!e.is_final);
        } else {
            panic!("expected StatusUpdate");
        }
    }

    #[test]
    fn status_update_final_flag() {
        let event = StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "t".to_string(),
            context_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            is_final: true,
            metadata: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["final"], true);
    }

    #[test]
    fn artifact_update_round_trip() {
        let event = make_artifact_update();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["kind"], "artifact-update");
        if let StreamEvent::ArtifactUpdate(ref e) = parsed {
            assert_eq!(e.task_id, "task-1");
            assert_eq!(e.artifact.artifact_id, "art-1");
            assert!(e.last_chunk);
            assert!(!e.append);
        } else {
            panic!("expected ArtifactUpdate");
        }
    }

    #[test]
    fn task_event_serializes_with_kind_tag() {
        // Task and Message already have a `kind` field in their struct,
        // which conflicts with StreamEvent's serde tag on deserialization.
        // We test serialization produces the correct kind tag.
        let event = make_task_event();
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], "task");
        assert_eq!(value["id"], "task-1");
        assert_eq!(value["status"]["state"], "completed");
    }

    #[test]
    fn message_event_serializes_with_kind_tag() {
        let event = make_message_event();
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], "message");
        assert_eq!(value["messageId"], "msg-1");
        assert_eq!(value["role"], "agent");
    }

    #[test]
    fn all_kind_discriminators() {
        let events = vec![
            (make_status_update(), "status-update"),
            (make_artifact_update(), "artifact-update"),
            (make_task_event(), "task"),
            (make_message_event(), "message"),
        ];
        for (event, expected_kind) in events {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(
                value["kind"].as_str().unwrap(),
                expected_kind,
                "kind discriminator for {expected_kind}"
            );
        }
    }

    // ============================================================================
    // mika#1731 — ToolCallStart / ToolCallResult / Unknown catch-all
    // ============================================================================

    fn make_tool_call_start() -> StreamEvent {
        StreamEvent::ToolCallStart(ToolCallStartEvent {
            task_id: "task-1".to_string(),
            context_id: Some("ctx-1".to_string()),
            step: 2,
            tool_name: "run_gh".to_string(),
            args_summary: "{\"subcommand\":\"issue\",\"args\":[\"view\",\"1731\"]}".to_string(),
            timestamp: "2026-07-10T13:00:00Z".to_string(),
        })
    }

    fn make_tool_call_result() -> StreamEvent {
        StreamEvent::ToolCallResult(ToolCallResultEvent {
            task_id: "task-1".to_string(),
            context_id: Some("ctx-1".to_string()),
            step: 2,
            tool_name: "run_gh".to_string(),
            success: true,
            non_zero_exit: false,
            output_summary: "{\"number\":1731,\"state\":\"OPEN\"}".to_string(),
            duration_ms: 123,
            timestamp: "2026-07-10T13:00:00Z".to_string(),
        })
    }

    #[test]
    fn tool_call_start_round_trip_preserves_fields() {
        let event = make_tool_call_start();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            StreamEvent::ToolCallStart(e) => {
                assert_eq!(e.task_id, "task-1");
                assert_eq!(e.context_id, Some("ctx-1".to_string()));
                assert_eq!(e.step, 2);
                assert_eq!(e.tool_name, "run_gh");
                assert!(e.args_summary.contains("issue"));
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_result_round_trip_preserves_fields() {
        let event = make_tool_call_result();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            StreamEvent::ToolCallResult(e) => {
                assert_eq!(e.tool_name, "run_gh");
                assert!(e.success);
                assert!(!e.non_zero_exit);
                assert_eq!(e.duration_ms, 123);
            }
            other => panic!("expected ToolCallResult, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_start_kind_tag() {
        let value = serde_json::to_value(make_tool_call_start()).unwrap();
        assert_eq!(value["kind"], "tool-call-start");
        assert_eq!(value["taskId"], "task-1");
        assert_eq!(value["toolName"], "run_gh");
        assert_eq!(value["step"], 2);
    }

    #[test]
    fn tool_call_result_kind_tag() {
        let value = serde_json::to_value(make_tool_call_result()).unwrap();
        assert_eq!(value["kind"], "tool-call-result");
        assert_eq!(value["success"], true);
        assert_eq!(value["nonZeroExit"], false);
        assert_eq!(value["durationMs"], 123);
    }

    #[test]
    fn unknown_kind_falls_to_catch_all_variant() {
        // A future release ships a new frame type. Older consumers must
        // deserialize without panic — they land in `Unknown` and can be
        // ignored or logged.
        let future_frame = serde_json::json!({
            "kind": "future-frame-v2",
            "taskId": "task-1",
            "some_field": 42,
        });
        let parsed: StreamEvent = serde_json::from_value(future_frame).unwrap();
        matches!(parsed, StreamEvent::Unknown);
    }

    #[test]
    fn truncate_tool_summary_below_cap_untouched() {
        let s = "short";
        assert_eq!(truncate_tool_summary(s), "short");
    }

    #[test]
    fn truncate_tool_summary_over_cap_gets_ellipsis() {
        let s = "a".repeat(TOOL_CALL_SUMMARY_CAP_CHARS + 10);
        let out = truncate_tool_summary(&s);
        assert_eq!(out.chars().count(), TOOL_CALL_SUMMARY_CAP_CHARS + 1); // + ellipsis
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_tool_summary_utf8_safe() {
        // Multi-byte codepoints must not be split at a byte boundary
        // (guards against the mika#764 lint class of bugs).
        let s = "é".repeat(TOOL_CALL_SUMMARY_CAP_CHARS + 10);
        let out = truncate_tool_summary(&s);
        assert_eq!(out.chars().count(), TOOL_CALL_SUMMARY_CAP_CHARS + 1);
    }
}
