//! Permission-decision request stream (mika#1733 sub-C AC1).
//!
//! Wire protocol between mika-spirit's permission classifier and the TUI /
//! dashboard: when the classifier defers a tool call for operator approval,
//! the request is broadcast on an SSE channel and the response POST-back
//! correlates by `request_id`.
//!
//! **Design contract**: `crates/mika-agent/docs/permission-decision-protocol-
//! 2026-07-06.md § AC1`. Any divergence from that doc halts + routes through
//! samidarko outbox per the ratification-preservation clause.
//!
//! **Discriminated union with sub-D (AskUserQuestion, mika#1734)**: this
//! channel carries two event shapes — `permission_request` and
//! `ask_user_question` — sharing the same connection + auth surface. sub-D
//! adds its variant + companion answer endpoint in a follow-up PR.
//!
//! **AC3 wire-schema rejection**: `PermissionDecideRequest` uses
//! `#[serde(deny_unknown_fields)]`. Any body containing `decision_authority`
//! (or any other unrecognized key) returns 400 — server-side config MUST
//! NEVER be wire-carried input.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};
use uuid::Uuid;

use super::state::AppState;

/// Bounded broadcast capacity — slow consumers drop-oldest per AC1.4.
/// Emission NEVER blocks on subscriber-slow behavior (see cm event-bus
/// discipline). 128 is empirical: at peak observed operator-decision
/// throughput (~1/s), 128 buffers 2 minutes of held requests before drop.
const CHANNEL_CAP: usize = 128;

/// Server-side default for held-request timeout. Overridable via
/// `MIKA_PERMISSION_HOLD_TIMEOUT_SECS`. Fail-closed: timeout materializes an
/// internal `deny` — the operator's absence never approves.
pub const DEFAULT_HOLD_TIMEOUT_SECS: u64 = 300;

// ── Wire types ────────────────────────────────────────────────────────────

/// Frame emitted on the SSE stream. Discriminated by the outer `event:`
/// field (via serde's `tag = "event"`); each variant carries its own
/// correlation `request_id` for POST-back matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PermissionStreamFrame {
    PermissionRequest {
        request_id: Uuid,
        tool_name: String,
        args_summary: String,
        classifier_verdict: ClassifierVerdict,
        held_reason: String,
    },
    /// Reserved for sub-D (mika#1734). Shape kept here so consumers can
    /// discriminate on the single channel; the emit-side handler lands in
    /// sub-D's follow-up PR.
    AskUserQuestion {
        request_id: Uuid,
        questions: serde_json::Value,
    },
    /// AC1.4 overflow marker — signals to the consumer that at least one
    /// frame was dropped due to slow-consumer backpressure.
    OverflowMarker { dropped_count: u64 },
}

/// AC2 provenance field: verbatim what the classifier said.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierVerdict {
    Approved,
    Denied,
    Held,
}

/// POST-back body. `deny_unknown_fields` enforces AC3: `decision_authority`
/// is NEVER a valid input; server-side config MUST NEVER be wire-carried.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDecideRequest {
    pub decision: OperatorDecision,
    #[serde(default)]
    pub reason: Option<String>,
}

/// AC2 provenance field: what the operator ratified. `None` at record time
/// = no decision required (classifier auto-approved without escalation).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorDecision {
    Approve,
    Deny,
}

// ── Shared channel state ──────────────────────────────────────────────────

/// Per-server permission-decision coordination surface. One instance lives
/// on `AppState`; the classifier holds a handle to it and pushes requests
/// via [`Self::send_request`]; the SSE handler subscribes; the POST-back
/// handler routes decisions back via [`Self::resolve_decision`].
pub struct PermissionsChannel {
    /// Broadcast channel for outgoing request frames. Multiple TUI clients
    /// can subscribe; each gets an independent slow-consumer envelope.
    outgoing: broadcast::Sender<PermissionStreamFrame>,
    /// Pending decisions awaiting operator resolution. Keyed by
    /// `request_id`. The `oneshot::Sender` fires when the POST-back handler
    /// receives a matching decide payload — the classifier awaits on the
    /// paired receiver.
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<OperatorDecision>>>>,
}

impl PermissionsChannel {
    pub fn new() -> Self {
        let (outgoing, _) = broadcast::channel(CHANNEL_CAP);
        Self {
            outgoing,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Broadcast a permission-request frame to all connected SSE
    /// subscribers. Returns `false` when zero subscribers are connected
    /// (informational only — the request is still registered in `pending`
    /// so a subscriber that connects later can resume it via cursor
    /// replay, once implemented).
    pub fn broadcast_frame(&self, frame: PermissionStreamFrame) -> bool {
        self.outgoing.send(frame).is_ok()
    }

    /// Register a pending decision request. Returns the `oneshot::Receiver`
    /// the caller awaits on. Timeout wrapping is the caller's
    /// responsibility (default: `DEFAULT_HOLD_TIMEOUT_SECS`).
    pub async fn register_pending(&self, request_id: Uuid) -> oneshot::Receiver<OperatorDecision> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(request_id, tx);
        rx
    }

    /// Route a POST-back decision to the classifier's oneshot receiver.
    /// Returns `Err(())` when no pending request matches `request_id`
    /// (client sent a decision for a request that never existed or was
    /// already resolved).
    pub async fn resolve_decision(
        &self,
        request_id: Uuid,
        decision: OperatorDecision,
    ) -> Result<(), ResolveError> {
        let mut pending = self.pending.lock().await;
        let sender = pending
            .remove(&request_id)
            .ok_or(ResolveError::UnknownRequest)?;
        sender
            .send(decision)
            .map_err(|_| ResolveError::ClassifierDropped)?;
        Ok(())
    }

    /// Test-only accessor for the broadcast sender count. Non-test callers
    /// SHOULD NOT depend on subscriber presence for correctness.
    #[cfg(test)]
    pub fn receiver_count(&self) -> usize {
        self.outgoing.receiver_count()
    }
}

impl Default for PermissionsChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum ResolveError {
    /// No pending request matched the given `request_id`. Returned as 404
    /// to the POST-back caller.
    UnknownRequest,
    /// The classifier's oneshot receiver was dropped (agent loop moved on).
    /// Returned as 409 Conflict — the decision arrived too late.
    ClassifierDropped,
}

// ── HTTP handlers ─────────────────────────────────────────────────────────

/// GET /api/v1/dashboard/permissions/stream — SSE channel for
/// permission-request + ask_user_question frames (sub-C AC1 + sub-D).
///
/// Auth: bearer via `MIKA_INTERNAL_TOKEN` OR `MIKA_DASHBOARD_TOKEN` — wired
/// at route-registration time in `server::mod.rs` alongside sibling
/// dashboard SSE endpoints.
pub async fn handle_permissions_stream(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.permissions_channel.outgoing.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(frame) => match serde_json::to_string(&frame) {
            Ok(json) => Some(Ok::<_, std::convert::Infallible>(
                Event::default().data(json),
            )),
            Err(e) => {
                warn!(error = %e, "failed to serialize permission stream frame");
                None
            }
        },
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(dropped)) => {
            // AC1.4: on slow-consumer overflow emit a marker frame and
            // resume — never crash the stream. `dropped` is the count
            // since the last successful recv.
            let overflow = PermissionStreamFrame::OverflowMarker {
                dropped_count: dropped,
            };
            serde_json::to_string(&overflow)
                .ok()
                .map(|json| Ok(Event::default().data(json)))
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// POST /api/v1/dashboard/permissions/{request_id}/decide — operator
/// POST-back handler. Correlates via `request_id`; returns 404 on unknown
/// request, 409 on late decision (classifier dropped), 400 on unknown
/// field in body (AC3).
pub async fn handle_permission_decide(
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
    Json(body): Json<PermissionDecideRequest>,
) -> impl IntoResponse {
    match state
        .permissions_channel
        .resolve_decision(request_id, body.decision)
        .await
    {
        Ok(()) => {
            debug!(
                request_id = %request_id,
                decision = ?body.decision,
                "permission decision routed to classifier"
            );
            (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
        }
        Err(ResolveError::UnknownRequest) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "unknown_request",
                "request_id": request_id,
            })),
        )
            .into_response(),
        Err(ResolveError::ClassifierDropped) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "classifier_dropped",
                "detail": "the classifier moved on before the decision arrived (held-request timeout may have fired)"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC3: `decision_authority` in the POST body → 400 unknown_field.
    #[test]
    fn decide_request_rejects_decision_authority_field() {
        let body = r#"{"decision":"approve","decision_authority":"operator_override"}"#;
        let err = serde_json::from_str::<PermissionDecideRequest>(body).unwrap_err();
        assert!(
            err.to_string().contains("decision_authority"),
            "AC3 wire-schema rejection: `decision_authority` must be an unknown field. Got: {err}"
        );
    }

    /// AC3: also reject arbitrary unrecognized keys — deny_unknown_fields is
    /// closed-world.
    #[test]
    fn decide_request_rejects_arbitrary_unknown_field() {
        let body = r#"{"decision":"deny","foo":"bar"}"#;
        let err = serde_json::from_str::<PermissionDecideRequest>(body).unwrap_err();
        assert!(err.to_string().contains("foo"));
    }

    /// Happy-path: minimal valid body parses.
    #[test]
    fn decide_request_minimal_body_parses() {
        let body = r#"{"decision":"approve"}"#;
        let parsed: PermissionDecideRequest = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.decision, OperatorDecision::Approve);
        assert_eq!(parsed.reason, None);
    }

    /// With optional reason.
    #[test]
    fn decide_request_with_reason_parses() {
        let body = r#"{"decision":"deny","reason":"unsafe path"}"#;
        let parsed: PermissionDecideRequest = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.decision, OperatorDecision::Deny);
        assert_eq!(parsed.reason.as_deref(), Some("unsafe path"));
    }

    /// Channel resolve routes the decision to the classifier's receiver.
    #[tokio::test]
    async fn channel_resolves_pending_decision() {
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch.register_pending(request_id).await;

        ch.resolve_decision(request_id, OperatorDecision::Approve)
            .await
            .expect("resolve should succeed");

        let decision = rx.await.expect("classifier receives decision");
        assert_eq!(decision, OperatorDecision::Approve);
    }

    /// Resolve for an unknown request_id → UnknownRequest.
    #[tokio::test]
    async fn channel_rejects_unknown_request_id() {
        let ch = PermissionsChannel::new();
        let err = ch
            .resolve_decision(Uuid::new_v4(), OperatorDecision::Approve)
            .await
            .expect_err("unknown request must fail");
        matches!(err, ResolveError::UnknownRequest);
    }

    /// If the classifier's receiver is dropped before the operator answers,
    /// the resolve returns ClassifierDropped (409).
    #[tokio::test]
    async fn channel_reports_classifier_dropped() {
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch.register_pending(request_id).await;
        drop(rx);

        let err = ch
            .resolve_decision(request_id, OperatorDecision::Deny)
            .await
            .expect_err("dropped receiver must fail");
        matches!(err, ResolveError::ClassifierDropped);
    }

    /// SSE broadcast: send a frame, subscriber receives it.
    #[tokio::test]
    async fn broadcast_frame_reaches_subscriber() {
        let ch = PermissionsChannel::new();
        let mut rx = ch.outgoing.subscribe();
        let request_id = Uuid::new_v4();
        let frame = PermissionStreamFrame::PermissionRequest {
            request_id,
            tool_name: "Bash".to_string(),
            args_summary: "git status".to_string(),
            classifier_verdict: ClassifierVerdict::Held,
            held_reason: "requires operator review".to_string(),
        };
        assert!(ch.broadcast_frame(frame.clone()));

        let received = rx.recv().await.expect("subscriber receives frame");
        match received {
            PermissionStreamFrame::PermissionRequest { tool_name, .. } => {
                assert_eq!(tool_name, "Bash");
            }
            other => panic!("expected PermissionRequest variant, got {other:?}"),
        }
    }
}
