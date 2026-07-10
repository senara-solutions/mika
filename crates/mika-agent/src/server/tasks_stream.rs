//! Task-event live stream for the TUI status pane (mika#1732 sub-B).
//!
//! Wire protocol between mika-spirit's task engine and the TUI / dashboard:
//! every task lifecycle transition (created / claimed / completed / failed /
//! delivered / cancelled) is broadcast on an SSE channel. Consumers see live
//! task state changes without polling `GET /api/v1/tasks`.
//!
//! **Wire-first split (mika#1732 v1).** This module ships the SSE surface —
//! the enum, the channel primitive, the handler, and the route contract.
//! Emission from `task_engine` transition sites is deferred to a follow-up
//! ticket; the wire is dormant until that lands. Same pattern as mika#1741
//! (`permissions_stream.rs`) and mika#1731 (`mika_a2a::streaming`).
//!
//! **Sibling divergence — do not conflate.** Three sibling SSE surfaces
//! now coexist, each with a deliberate choice of discriminator key + scope:
//!
//! | Surface                                | Discriminator | Scope          | Route pattern                    |
//! |----------------------------------------|---------------|----------------|----------------------------------|
//! | `mika_a2a::streaming::StreamEvent`     | `tag = "kind"`  | Per-task       | JSON-RPC-inline SSE              |
//! | `PermissionStreamFrame` (mika#1741)    | `tag = "event"` | Per-process    | `GET /dashboard/permissions/stream` |
//! | `TaskEventFrame` (this file, mika#1732)| `tag = "event"` | Per-process    | `GET /dashboard/tasks/stream`    |
//!
//! The A2A vs Dashboard family divergence is intentional: A2A is JSON-RPC
//! transport with task-scoped sessions; Dashboard is HTTP SSE with global
//! tenant-scoped streams. `TaskEventFrame` follows `PermissionStreamFrame`
//! because it lives in the same Dashboard family, not because the
//! discriminator key was arbitrary.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use super::state::AppState;

/// Bounded broadcast capacity — slow consumers drop-oldest per the ticket's
/// AC3. Emission NEVER blocks on subscriber-slow behavior. 256 is double the
/// permissions channel cap because task-transition traffic can burst higher
/// on dispatch cascades and callback-delivery flurries.
const CHANNEL_CAP: usize = 256;

/// UTF-8-safe truncation cap for `result_preview` / `error_preview` fields.
/// Frames are glanceable receipts; the full call remains queryable via
/// `GET /api/v1/tasks/{task_id}` for the details view.
const PREVIEW_CAP_CHARS: usize = 500;

/// UTF-8-safe truncation for preview fields. Cuts at a codepoint boundary
/// and appends U+2026 when truncation actually removed characters. Guards
/// against the mika#764 byte-slice lint class.
fn truncate_preview(s: &str) -> String {
    if s.chars().count() <= PREVIEW_CAP_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(PREVIEW_CAP_CHARS).collect();
    out.push('…');
    out
}

// ── Wire types ────────────────────────────────────────────────────────────

/// Frame emitted on the SSE stream. Discriminated by the outer `event:`
/// field (via serde's `tag = "event"`). Each transition variant carries the
/// task and agent identifiers so consumers can correlate against
/// `GET /api/v1/tasks/{task_id}` for the full record.
///
/// Consumers MUST tolerate unknown `event` values via the `Unknown`
/// catch-all — new variants may ship additively (mika#1734 permission
/// bridge, mika#1736 session-messages stream both anticipated).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEventFrame {
    /// A new task row was written to `tasks`. Fired on `db.create_task()`.
    TaskCreated {
        task_id: String,
        agent_id: String,
        /// `trigger_type` — one of `manual` / `callback` / `recurring` / `a2a`.
        kind: String,
        /// Action type (e.g. `resume_agent`, `send_message`, `none`).
        action_type: String,
        label: Option<String>,
        parent_task_id: Option<String>,
        /// ISO 8601 UTC.
        created_at: String,
    },
    /// A task transitioned to `in_progress` (dispatch fired).
    TaskClaimed {
        task_id: String,
        agent_id: String,
        claimed_at: String,
    },
    /// A task transitioned to `completed`.
    TaskCompleted {
        task_id: String,
        agent_id: String,
        completed_at: String,
        /// Truncated preview of the task result (500-char cap, UTF-8 safe).
        /// Full body remains queryable via `GET /api/v1/tasks/{task_id}`.
        result_preview: Option<String>,
    },
    /// A task transitioned to `failed`.
    TaskFailed {
        task_id: String,
        agent_id: String,
        failed_at: String,
        /// Truncated preview of the error summary (500-char cap, UTF-8 safe).
        error_preview: Option<String>,
    },
    /// A completed / failed callback was consumed by the resume dispatcher.
    TaskDelivered {
        task_id: String,
        agent_id: String,
        delivered_at: String,
    },
    /// A task transitioned to `cancelled` (operator or engine-driven).
    TaskCancelled {
        task_id: String,
        agent_id: String,
        cancelled_at: String,
        reason: Option<String>,
    },
    /// Emitted by the SSE handler when `BroadcastStream` reports
    /// `Lagged(n)` — the client fell behind and `n` frames were dropped.
    /// Same discipline as `PermissionStreamFrame::OverflowMarker`.
    OverflowMarker { dropped_count: u64 },
    /// Forward-compat catch-all. Consumers should match `Unknown` and
    /// fall through with a log-and-ignore branch. New variants will land
    /// in follow-up tickets (mika#1727 TUI subscriber refactor, etc.).
    #[serde(other)]
    Unknown,
}

impl TaskEventFrame {
    /// Helper: build a `TaskCompleted` frame with the result field
    /// UTF-8-safe truncated. Emission-follow-up sites call this rather
    /// than the struct literal so the truncation invariant is enforced.
    pub fn completed(
        task_id: impl Into<String>,
        agent_id: impl Into<String>,
        completed_at: impl Into<String>,
        result: Option<&str>,
    ) -> Self {
        Self::TaskCompleted {
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            completed_at: completed_at.into(),
            result_preview: result.map(truncate_preview),
        }
    }

    /// Helper: build a `TaskFailed` frame with the error field
    /// UTF-8-safe truncated.
    pub fn failed(
        task_id: impl Into<String>,
        agent_id: impl Into<String>,
        failed_at: impl Into<String>,
        error: Option<&str>,
    ) -> Self {
        Self::TaskFailed {
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            failed_at: failed_at.into(),
            error_preview: error.map(truncate_preview),
        }
    }
}

// ── Channel primitive ─────────────────────────────────────────────────────

/// Per-server task-event broadcast surface. One instance lives on
/// `AppState.task_events_channel`; task-engine transition sites push frames
/// via [`Self::broadcast_frame`]; the SSE handler subscribes via
/// [`Self::subscribe`].
///
/// The channel is per-process, not per-agent. Frames carry `agent_id` so
/// consumers can filter client-side. Single-tenant deploy shape makes this
/// the right default; a per-agent `DashMap<AgentId, Sender>` upgrade path
/// is documented in the emission-follow-up ticket if multi-tenant becomes
/// real (see mika#1732 plan §Risks).
pub struct TaskEventsChannel {
    outgoing: broadcast::Sender<TaskEventFrame>,
}

impl TaskEventsChannel {
    pub fn new() -> Self {
        let (outgoing, _) = broadcast::channel(CHANNEL_CAP);
        Self { outgoing }
    }

    /// Broadcast a task-event frame to all connected SSE subscribers.
    /// Returns `false` when zero subscribers are connected (informational
    /// only — the wire is dormant until a client connects; no cursor
    /// replay is implemented for v1).
    ///
    /// Emission is fire-and-forget: an error here NEVER fails the caller's
    /// DB write. Log at `debug!` if you want visibility during rollout.
    pub fn broadcast_frame(&self, frame: TaskEventFrame) -> bool {
        self.outgoing.send(frame).is_ok()
    }

    /// Subscribe to the stream. Used by the SSE handler; each subscription
    /// gets its own bounded receiver with drop-oldest slow-consumer
    /// behavior (translated to `OverflowMarker` on the wire).
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEventFrame> {
        self.outgoing.subscribe()
    }

    /// Test-only accessor. Non-test callers SHOULD NOT depend on subscriber
    /// presence for correctness — emission is fire-and-forget by contract.
    #[cfg(test)]
    pub fn receiver_count(&self) -> usize {
        self.outgoing.receiver_count()
    }
}

impl Default for TaskEventsChannel {
    fn default() -> Self {
        Self::new()
    }
}

// ── HTTP handler ─────────────────────────────────────────────────────────

/// GET /api/v1/dashboard/tasks/stream — SSE channel for task lifecycle
/// events (mika#1732 sub-B).
///
/// Auth: bearer via `MIKA_INTERNAL_TOKEN` OR `MIKA_DASHBOARD_TOKEN` — wired
/// at route-registration time in `server::mod.rs` alongside sibling
/// dashboard SSE endpoints (`permissions_stream`).
///
/// Slow-consumer discipline (ticket AC3): `BroadcastStreamRecvError::Lagged`
/// is translated to a `TaskEventFrame::OverflowMarker { dropped_count }`
/// frame on the wire. The stream never crashes on lag — it emits the
/// marker and resumes.
pub async fn handle_tasks_events_stream(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.task_events_channel.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(frame) => match serde_json::to_string(&frame) {
            Ok(json) => Some(Ok::<_, std::convert::Infallible>(
                Event::default().data(json),
            )),
            Err(e) => {
                warn!(error = %e, "failed to serialize task event frame");
                None
            }
        },
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(dropped)) => {
            // Slow-consumer overflow — emit a marker frame and resume.
            let overflow = TaskEventFrame::OverflowMarker {
                dropped_count: dropped,
            };
            serde_json::to_string(&overflow)
                .ok()
                .map(|json| Ok(Event::default().data(json)))
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Serde round-trip tests — one per emitted variant. Guards against
    // silent wire-tag drift.

    #[test]
    fn task_created_round_trip_preserves_fields() {
        let frame = TaskEventFrame::TaskCreated {
            task_id: "task-1".to_string(),
            agent_id: "mika".to_string(),
            kind: "manual".to_string(),
            action_type: "none".to_string(),
            label: Some("Investigate mika#1732".to_string()),
            parent_task_id: None,
            created_at: "2026-07-10T13:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: TaskEventFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            TaskEventFrame::TaskCreated {
                task_id, agent_id, ..
            } => {
                assert_eq!(task_id, "task-1");
                assert_eq!(agent_id, "mika");
            }
            other => panic!("expected TaskCreated, got {other:?}"),
        }
    }

    #[test]
    fn task_completed_round_trip_with_truncated_result() {
        let big_result = "a".repeat(PREVIEW_CAP_CHARS + 50);
        let frame =
            TaskEventFrame::completed("task-2", "mika", "2026-07-10T13:01:00Z", Some(&big_result));
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: TaskEventFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            TaskEventFrame::TaskCompleted { result_preview, .. } => {
                let preview = result_preview.expect("result should be Some");
                // Truncated to PREVIEW_CAP_CHARS + 1 ellipsis codepoint.
                assert_eq!(preview.chars().count(), PREVIEW_CAP_CHARS + 1);
                assert!(preview.ends_with('…'));
            }
            other => panic!("expected TaskCompleted, got {other:?}"),
        }
    }

    #[test]
    fn task_failed_round_trip_with_truncated_error() {
        let big_error = "err ".repeat(PREVIEW_CAP_CHARS);
        let frame =
            TaskEventFrame::failed("task-3", "mika", "2026-07-10T13:02:00Z", Some(&big_error));
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: TaskEventFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            TaskEventFrame::TaskFailed { error_preview, .. } => {
                let preview = error_preview.expect("error should be Some");
                assert!(preview.chars().count() <= PREVIEW_CAP_CHARS + 1);
                assert!(preview.ends_with('…'));
            }
            other => panic!("expected TaskFailed, got {other:?}"),
        }
    }

    // Wire-tag stability tests — every variant's serde tag string is
    // asserted. Silent renames break subscribers, so pin the strings.

    #[test]
    fn wire_tag_strings_are_stable() {
        let cases: Vec<(TaskEventFrame, &str)> = vec![
            (
                TaskEventFrame::TaskCreated {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    kind: "manual".into(),
                    action_type: "none".into(),
                    label: None,
                    parent_task_id: None,
                    created_at: "ts".into(),
                },
                "task_created",
            ),
            (
                TaskEventFrame::TaskClaimed {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    claimed_at: "ts".into(),
                },
                "task_claimed",
            ),
            (
                TaskEventFrame::TaskCompleted {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    completed_at: "ts".into(),
                    result_preview: None,
                },
                "task_completed",
            ),
            (
                TaskEventFrame::TaskFailed {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    failed_at: "ts".into(),
                    error_preview: None,
                },
                "task_failed",
            ),
            (
                TaskEventFrame::TaskDelivered {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    delivered_at: "ts".into(),
                },
                "task_delivered",
            ),
            (
                TaskEventFrame::TaskCancelled {
                    task_id: "t".into(),
                    agent_id: "a".into(),
                    cancelled_at: "ts".into(),
                    reason: None,
                },
                "task_cancelled",
            ),
            (
                TaskEventFrame::OverflowMarker { dropped_count: 1 },
                "overflow_marker",
            ),
        ];
        for (frame, expected) in cases {
            let value = serde_json::to_value(&frame).unwrap();
            assert_eq!(
                value["event"].as_str().unwrap(),
                expected,
                "wire tag for {frame:?}"
            );
        }
    }

    // Forward-compat catch-all.

    #[test]
    fn unknown_event_falls_to_catch_all_variant() {
        let future = serde_json::json!({
            "event": "future_variant_not_yet_added",
            "task_id": "t",
            "extra": 42,
        });
        let parsed: TaskEventFrame = serde_json::from_value(future).unwrap();
        assert!(
            matches!(parsed, TaskEventFrame::Unknown),
            "unknown event should land in Unknown catch-all"
        );
    }

    // Channel mechanics.

    #[tokio::test]
    async fn broadcast_frame_reaches_subscriber() {
        let channel = TaskEventsChannel::new();
        let mut rx = channel.subscribe();
        let frame = TaskEventFrame::TaskDelivered {
            task_id: "t".into(),
            agent_id: "a".into(),
            delivered_at: "ts".into(),
        };
        assert!(channel.broadcast_frame(frame));
        let received = rx.recv().await.expect("should receive frame");
        match received {
            TaskEventFrame::TaskDelivered { task_id, .. } => assert_eq!(task_id, "t"),
            other => panic!("expected TaskDelivered, got {other:?}"),
        }
    }

    #[test]
    fn broadcast_frame_zero_subscribers_returns_false() {
        let channel = TaskEventsChannel::new();
        assert_eq!(channel.receiver_count(), 0);
        let frame = TaskEventFrame::OverflowMarker { dropped_count: 0 };
        // No subscribers — send returns Err internally; broadcast_frame
        // reports false but does NOT panic or fail the caller.
        assert!(!channel.broadcast_frame(frame));
    }

    #[test]
    fn truncate_preview_below_cap_untouched() {
        let s = "short preview";
        assert_eq!(truncate_preview(s), "short preview");
    }

    #[test]
    fn truncate_preview_over_cap_appends_ellipsis() {
        let s = "a".repeat(PREVIEW_CAP_CHARS + 10);
        let out = truncate_preview(&s);
        assert_eq!(out.chars().count(), PREVIEW_CAP_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_preview_utf8_safe() {
        // Multi-byte codepoints must not split at a byte boundary
        // (mika#764 lint class).
        let s = "é".repeat(PREVIEW_CAP_CHARS + 10);
        let out = truncate_preview(&s);
        assert_eq!(out.chars().count(), PREVIEW_CAP_CHARS + 1);
    }
}
