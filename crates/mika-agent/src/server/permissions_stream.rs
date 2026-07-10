//! Permission-decision request stream (mika#1733 sub-C AC1-AC8).
//!
//! Wire protocol between mika-spirit's permission classifier and the TUI /
//! dashboard: when the classifier defers a tool call for operator approval,
//! the request is broadcast on an SSE channel and the response POST-back
//! correlates by `request_id`. Every resolved decision persists a
//! provenance row to `permission_decisions` (AC4) so ratification is
//! auditable independent of live SSE consumers.
//!
//! **Design contract**: `crates/mika-agent/docs/permission-decision-protocol-
//! 2026-07-06.md § AC1-§AC8`. Any divergence from that doc halts + routes
//! through samidarko outbox per the ratification-preservation clause.
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
use mika_common::config::DecisionAuthority;
use mika_common::permission_authority::{DecisionScope, resolve_authority};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{debug, warn};
use uuid::Uuid;

use super::state::AppState;
use crate::async_db::AsyncDatabase;

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
    /// Structured multi-choice question routed to the operator (mika#1734
    /// sub-D). Shares the SSE channel and auth surface with
    /// `PermissionRequest` via the discriminated-union pattern; the answer
    /// path is a sibling POST endpoint (`/answer`), not `/decide`.
    AskUserQuestion {
        request_id: Uuid,
        questions: Vec<AskQuestion>,
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

impl ClassifierVerdict {
    /// Storage form written to the `permission_decisions.classifier_verdict`
    /// column (matches the CHECK constraint in v44).
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Held => "held",
        }
    }
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

impl OperatorDecision {
    /// Storage form written to the
    /// `permission_decisions.operator_decision` column.
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }
}

/// Structured multi-choice question payload for AskUserQuestion (mika#1734).
///
/// Mirrors the shape used by `AskUserQuestion` in `claude-agent-sdk` so the
/// TUI-side renderer can consume both surfaces uniformly. `multi_select`
/// serializes as camelCase `multiSelect` per the ticket contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AskQuestion {
    pub question: String,
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AskOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// POST-back body for `AskUserQuestion` (mika#1734 AC2). Answers are keyed
/// by question **index** (`"0"`, `"1"`, …) so the wire format is stable
/// against question-text edits inside a single request lifetime.
/// `deny_unknown_fields` matches the discipline of the decide endpoint.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionAnswerRequest {
    pub answers: HashMap<String, String>,
}

/// Result delivered to the classifier's oneshot when an
/// [`PermissionsChannel::register_pending_ask`] slot resolves.
///
/// - `Answered`: operator POSTed valid answers before the timeout.
/// - `Timeout`: server-side hold expired; the agent loop must handle a
///   `Deny { reason: "operator-timeout" }`-shaped continuation per the
///   claude-pilot cpp#20 joint-2 discipline referenced in the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerResult {
    Answered { answers: HashMap<String, String> },
    Timeout { reason: String },
}

impl AnswerResult {
    pub fn operator_timeout() -> Self {
        Self::Timeout {
            reason: "operator-timeout".to_string(),
        }
    }
}

/// AC4 provenance metadata registered at classifier-emit time and looked up
/// by the POST-back handler to build the full [`PermissionsChannel::resolve_decision`]
/// call. Snapshot form is intentionally `Clone` so the handler can inspect
/// pending state without holding the `PermissionsChannel::pending` lock.
#[derive(Debug, Clone)]
pub struct PendingRequestSnapshot {
    pub classifier_verdict: ClassifierVerdict,
    pub tool_name: String,
    pub args_summary: Option<String>,
}

/// Internal storage entry for a pending permission request. Carries the
/// classifier's oneshot sender alongside the provenance snapshot so the
/// POST-back handler can reconstruct the full decision context under a
/// single atomic remove-and-fire.
struct PendingRequest {
    sender: oneshot::Sender<OperatorDecision>,
    classifier_verdict: ClassifierVerdict,
    tool_name: String,
    args_summary: Option<String>,
}

/// Internal storage entry for a pending `AskUserQuestion` (mika#1734). Holds
/// the classifier's oneshot sender + the question set so the POST-back
/// handler can validate answers against the original options.
struct PendingAsk {
    sender: oneshot::Sender<AnswerResult>,
    questions: Vec<AskQuestion>,
}

// ── Shared channel state ──────────────────────────────────────────────────

/// Per-server permission-decision coordination surface. One instance lives
/// on `AppState`; the classifier holds a handle to it and pushes requests
/// via [`Self::broadcast_frame`] + [`Self::register_pending`]; the SSE handler
/// subscribes; the POST-back handler routes decisions back via
/// [`Self::resolve_decision`].
pub struct PermissionsChannel {
    /// Broadcast channel for outgoing request frames. Multiple TUI clients
    /// can subscribe; each gets an independent slow-consumer envelope.
    outgoing: broadcast::Sender<PermissionStreamFrame>,
    /// Pending decisions awaiting operator resolution. Keyed by
    /// `request_id`. Each entry carries the classifier's oneshot sender +
    /// provenance snapshot; the POST-back handler removes the entry, fires
    /// the sender, and consumes the snapshot to build the DB provenance row.
    pending: Arc<Mutex<HashMap<Uuid, PendingRequest>>>,
    /// Pending `AskUserQuestion` requests (mika#1734). Sibling map to
    /// `pending`; separate because the resolution shape is different
    /// (`AnswerResult` vs `OperatorDecision`). Same discriminated-union
    /// SSE channel + auth surface — this map is purely internal state.
    pending_asks: Arc<Mutex<HashMap<Uuid, PendingAsk>>>,
}

impl PermissionsChannel {
    pub fn new() -> Self {
        let (outgoing, _) = broadcast::channel(CHANNEL_CAP);
        Self {
            outgoing,
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_asks: Arc::new(Mutex::new(HashMap::new())),
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
    ///
    /// Provenance metadata is registered alongside the sender so the
    /// POST-back handler can build the [`Self::resolve_decision`] call
    /// without a separate lookup surface.
    pub async fn register_pending(
        &self,
        request_id: Uuid,
        classifier_verdict: ClassifierVerdict,
        tool_name: String,
        args_summary: Option<String>,
    ) -> oneshot::Receiver<OperatorDecision> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            request_id,
            PendingRequest {
                sender: tx,
                classifier_verdict,
                tool_name,
                args_summary,
            },
        );
        rx
    }

    /// Read a snapshot of a pending request's provenance metadata without
    /// removing the entry. Used by the POST-back handler to build the full
    /// [`Self::resolve_decision`] call; a benign race with a concurrent
    /// decide on the same `request_id` produces a 404 on the losing peek's
    /// eventual resolve, which is the correct semantics.
    pub async fn peek_pending(&self, request_id: Uuid) -> Option<PendingRequestSnapshot> {
        self.pending
            .lock()
            .await
            .get(&request_id)
            .map(|p| PendingRequestSnapshot {
                classifier_verdict: p.classifier_verdict,
                tool_name: p.tool_name.clone(),
                args_summary: p.args_summary.clone(),
            })
    }

    /// Route a POST-back decision to the classifier's oneshot receiver AND
    /// persist a provenance row to `permission_decisions` (AC4).
    ///
    /// **Ordering discipline (AC4)**: the oneshot fires FIRST — the
    /// classifier's continue-or-deny path is unblocked before the DB write
    /// runs. The DB write is spawned onto the tokio runtime so a slow DB
    /// never bleeds into the classifier's ≤500ms budget. If `db` is `None`
    /// (test contexts without a wired DB) the persistence step is a no-op.
    ///
    /// **`override_used` derivation (AC4)**: `true` iff
    /// `classifier_verdict == Denied && decision == Approve && authority ==
    /// Override`. Under `Strict` authority `override_used` is ALWAYS
    /// `false` — the operator's decision is advisory only per AC8.
    ///
    /// **Signature note (mika-arch F1)**: single function, no wrapper. The
    /// provenance params (`classifier_verdict`, `tool_name`, `args_summary`)
    /// are threaded through explicitly so every value persisted is visible
    /// at the call site — no silent-data-loss seam.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_decision(
        &self,
        db: Option<&AsyncDatabase>,
        request_id: Uuid,
        classifier_verdict: ClassifierVerdict,
        decision: OperatorDecision,
        tool_name: &str,
        args_summary: Option<&str>,
        authority: DecisionAuthority,
        scope: DecisionScope,
    ) -> Result<(), ResolveError> {
        let sender = {
            let mut pending = self.pending.lock().await;
            pending
                .remove(&request_id)
                .ok_or(ResolveError::UnknownRequest)?
                .sender
        };
        // AC4 discipline: fire the classifier oneshot BEFORE the DB write.
        sender
            .send(decision)
            .map_err(|_| ResolveError::ClassifierDropped)?;

        if let Some(db) = db {
            let override_used = classifier_verdict == ClassifierVerdict::Denied
                && decision == OperatorDecision::Approve
                && authority == DecisionAuthority::Override;

            let db_clone = db.clone();
            let id = Uuid::new_v4().to_string();
            let request_id_str = request_id.to_string();
            let tool_name_owned = tool_name.to_string();
            let args_summary_owned = args_summary.map(str::to_string);
            let verdict_str = classifier_verdict.as_db_str().to_string();
            let decision_str = decision.as_db_str().to_string();
            let authority_str = match authority {
                DecisionAuthority::Strict => "strict".to_string(),
                DecisionAuthority::Override => "override".to_string(),
            };
            let tenant_id = scope.tenant_id.clone();
            let agent_id = scope.agent_id.clone();

            // Fire-and-forget from the classifier's perspective — the ≤500ms
            // budget applies to the oneshot, not the DB write. Errors are
            // logged; the classifier is not blocked or notified.
            tokio::spawn(async move {
                if let Err(e) = db_clone
                    .insert_permission_decision(
                        id,
                        request_id_str,
                        tool_name_owned,
                        args_summary_owned,
                        verdict_str,
                        Some(decision_str),
                        override_used,
                        authority_str,
                        tenant_id,
                        agent_id,
                    )
                    .await
                {
                    warn!(
                        error = %e,
                        request_id = %request_id,
                        "failed to persist permission_decisions row"
                    );
                }
            });
        }

        Ok(())
    }

    // ── AskUserQuestion (mika#1734) ──────────────────────────────────────

    /// Register a pending `AskUserQuestion` request and spawn a
    /// server-side hold-timeout (AC3). Returns the `oneshot::Receiver`
    /// the classifier awaits on.
    ///
    /// - On operator POST-back: [`Self::resolve_answer`] fires the sender
    ///   with `AnswerResult::Answered`.
    /// - On timeout: the spawned watcher fires the sender with
    ///   `AnswerResult::Timeout { reason: "operator-timeout" }` so the
    ///   agent loop can continue honestly (matches claude-pilot cpp#20
    ///   joint 2 discipline referenced in the ticket).
    ///
    /// `timeout` defaults to `Settings.permission_hold_timeout_secs`
    /// (mika#1733 AC3 already exposed the knob; #1734 reuses it verbatim
    /// per AC4).
    pub async fn register_pending_ask(
        self: &Arc<Self>,
        request_id: Uuid,
        questions: Vec<AskQuestion>,
        timeout: Duration,
    ) -> oneshot::Receiver<AnswerResult> {
        let (tx, rx) = oneshot::channel();
        self.pending_asks.lock().await.insert(
            request_id,
            PendingAsk {
                sender: tx,
                questions,
            },
        );

        // Spawn a hold-timeout watcher. If the pending entry is still
        // there at expiry, take the sender out and fire the timeout
        // outcome. A concurrent resolve_answer will have already removed
        // the entry, in which case the watcher is a no-op.
        let ch = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if let Some(entry) = ch.pending_asks.lock().await.remove(&request_id) {
                // Ignore send failure — the classifier may have dropped
                // its receiver.
                let _ = entry.sender.send(AnswerResult::operator_timeout());
                debug!(
                    request_id = %request_id,
                    "AskUserQuestion hold-timeout materialized operator-timeout"
                );
            }
        });

        rx
    }

    /// Read a snapshot of a pending `AskUserQuestion` without consuming.
    /// Symmetric with [`Self::peek_pending`] for
    /// `PermissionsChannel::PermissionRequest`.
    pub async fn peek_pending_ask(&self, request_id: Uuid) -> Option<Vec<AskQuestion>> {
        self.pending_asks
            .lock()
            .await
            .get(&request_id)
            .map(|p| p.questions.clone())
    }

    /// Route an operator's answers to the classifier's oneshot receiver.
    /// Validates that the `answers` map covers every question with a
    /// valid option label; on malformed input returns a structured
    /// [`AnswerError`] the handler renders as 400.
    pub async fn resolve_answer(
        &self,
        request_id: Uuid,
        answers: HashMap<String, String>,
    ) -> Result<(), AnswerError> {
        // Atomic remove-and-validate: take the entry out first so a
        // concurrent decide loses the race cleanly (returns UnknownRequest).
        let entry = {
            let mut pending = self.pending_asks.lock().await;
            pending
                .remove(&request_id)
                .ok_or(AnswerError::UnknownRequest)?
        };

        // Validate coverage: every question index must have a matching
        // answer, and each answer must be a valid option label.
        for (idx, question) in entry.questions.iter().enumerate() {
            let key = idx.to_string();
            let Some(answer) = answers.get(&key) else {
                // Put the entry back so a corrected retry can succeed.
                self.pending_asks.lock().await.insert(
                    request_id,
                    PendingAsk {
                        sender: entry.sender,
                        questions: entry.questions,
                    },
                );
                return Err(AnswerError::MissingAnswer {
                    question_index: idx,
                });
            };
            if !question.options.iter().any(|opt| opt.label == *answer) {
                self.pending_asks.lock().await.insert(
                    request_id,
                    PendingAsk {
                        sender: entry.sender,
                        questions: entry.questions,
                    },
                );
                return Err(AnswerError::InvalidOptionLabel {
                    question_index: idx,
                    supplied: answer.clone(),
                });
            }
        }

        // All questions covered with valid option labels. Reject any
        // extra keys that don't correspond to a question — closed-world
        // semantics match `deny_unknown_fields`.
        let expected_count = entry.questions.len();
        for key in answers.keys() {
            let Ok(idx) = key.parse::<usize>() else {
                self.pending_asks.lock().await.insert(
                    request_id,
                    PendingAsk {
                        sender: entry.sender,
                        questions: entry.questions,
                    },
                );
                return Err(AnswerError::ExtraKey { key: key.clone() });
            };
            if idx >= expected_count {
                self.pending_asks.lock().await.insert(
                    request_id,
                    PendingAsk {
                        sender: entry.sender,
                        questions: entry.questions,
                    },
                );
                return Err(AnswerError::ExtraKey { key: key.clone() });
            }
        }

        entry
            .sender
            .send(AnswerResult::Answered { answers })
            .map_err(|_| AnswerError::ClassifierDropped)?;
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

/// Validation and routing errors for `/answer` (mika#1734 AC2).
#[derive(Debug)]
pub enum AnswerError {
    /// 404 — no pending AskUserQuestion matches `request_id`.
    UnknownRequest,
    /// 400 — the answers map is missing an entry for `question_index`.
    MissingAnswer { question_index: usize },
    /// 400 — the answer for `question_index` is not among the declared
    /// option labels.
    InvalidOptionLabel {
        question_index: usize,
        supplied: String,
    },
    /// 400 — the answers map contains a key that does not correspond to
    /// any declared question index. Closed-world match.
    ExtraKey { key: String },
    /// 409 — the classifier's oneshot receiver was dropped before the
    /// operator answered. Rare; the agent loop moved on (usually because
    /// the hold-timeout already fired).
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
///
/// Handler flow (AC4, AC6):
/// 1. Peek pending metadata (classifier_verdict, tool_name, args_summary).
/// 2. Resolve effective authority via [`resolve_authority`] — reads the
///    per-agent → per-tenant → global env chain plus `Settings`.
/// 3. Call [`PermissionsChannel::resolve_decision`] with the full
///    provenance signature; that method atomically fires the oneshot and
///    spawns the DB write.
///
/// Scope note: `DecisionScope::global()` is used today because the tenant
/// / agent identifiers for a permission decision are not yet threaded from
/// the classifier's emit site. mika#1727 (parent) plumbs these through in
/// a follow-up; until then the global tier is the effective authority.
pub async fn handle_permission_decide(
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
    Json(body): Json<PermissionDecideRequest>,
) -> impl IntoResponse {
    let snapshot = match state.permissions_channel.peek_pending(request_id).await {
        Some(snapshot) => snapshot,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "unknown_request",
                    "request_id": request_id,
                })),
            )
                .into_response();
        }
    };

    let scope = DecisionScope::global();
    let authority = resolve_authority(&state.settings, &scope, |k| std::env::var(k).ok());

    match state
        .permissions_channel
        .resolve_decision(
            Some(&state.dashboard_db),
            request_id,
            snapshot.classifier_verdict,
            body.decision,
            &snapshot.tool_name,
            snapshot.args_summary.as_deref(),
            authority,
            scope,
        )
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

/// POST /api/v1/dashboard/permissions/{request_id}/answer — operator
/// answers-back handler for `AskUserQuestion` (mika#1734 AC2). Sibling of
/// [`handle_permission_decide`]: same auth surface, discriminated by
/// path suffix (`/answer` vs `/decide`).
///
/// Returns:
/// - 200 OK when the answer routes to the classifier.
/// - 400 Bad Request when the answers map is missing / has extras / has an
///   invalid option label / has an unparseable key. Body carries the
///   structured `AnswerError` shape.
/// - 404 Not Found when `request_id` is unknown or already resolved.
/// - 409 Conflict when the classifier's receiver was dropped.
pub async fn handle_permission_answer(
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
    Json(body): Json<PermissionAnswerRequest>,
) -> impl IntoResponse {
    match state
        .permissions_channel
        .resolve_answer(request_id, body.answers)
        .await
    {
        Ok(()) => {
            debug!(
                request_id = %request_id,
                "AskUserQuestion answer routed to classifier"
            );
            (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
        }
        Err(AnswerError::UnknownRequest) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "unknown_request",
                "request_id": request_id,
            })),
        )
            .into_response(),
        Err(AnswerError::MissingAnswer { question_index }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_answer",
                "question_index": question_index,
            })),
        )
            .into_response(),
        Err(AnswerError::InvalidOptionLabel {
            question_index,
            supplied,
        }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_option_label",
                "question_index": question_index,
                "supplied": supplied,
            })),
        )
            .into_response(),
        Err(AnswerError::ExtraKey { key }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "extra_key",
                "key": key,
            })),
        )
            .into_response(),
        Err(AnswerError::ClassifierDropped) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "classifier_dropped",
                "detail": "the classifier moved on before the answer arrived (held-request timeout may have fired)"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn scratch_db() -> AsyncDatabase {
        let db = Database::open_in_memory().expect("open in-memory DB");
        AsyncDatabase::new_with_agent(db, "test-agent")
    }

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
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Held,
                "Bash".to_string(),
                Some("git status".to_string()),
            )
            .await;

        ch.resolve_decision(
            None,
            request_id,
            ClassifierVerdict::Held,
            OperatorDecision::Approve,
            "Bash",
            Some("git status"),
            DecisionAuthority::Strict,
            DecisionScope::global(),
        )
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
            .resolve_decision(
                None,
                Uuid::new_v4(),
                ClassifierVerdict::Held,
                OperatorDecision::Approve,
                "Bash",
                None,
                DecisionAuthority::Strict,
                DecisionScope::global(),
            )
            .await
            .expect_err("unknown request must fail");
        assert!(matches!(err, ResolveError::UnknownRequest));
    }

    /// If the classifier's receiver is dropped before the operator answers,
    /// the resolve returns ClassifierDropped (409).
    #[tokio::test]
    async fn channel_reports_classifier_dropped() {
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Held,
                "Bash".to_string(),
                Some("git status".to_string()),
            )
            .await;
        drop(rx);

        let err = ch
            .resolve_decision(
                None,
                request_id,
                ClassifierVerdict::Held,
                OperatorDecision::Deny,
                "Bash",
                Some("git status"),
                DecisionAuthority::Strict,
                DecisionScope::global(),
            )
            .await
            .expect_err("dropped receiver must fail");
        assert!(matches!(err, ResolveError::ClassifierDropped));
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

    // ── AC4 provenance persistence ──────────────────────────────────────

    /// AC4 + AC8: Strict authority + classifier Denied + operator Approve
    /// → `override_used = 0`. The operator's decision is advisory only
    /// under the shipped default; the classifier verdict is what wins.
    #[tokio::test]
    async fn strict_authority_denied_approve_records_override_used_false() {
        let db = scratch_db().await;
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Denied,
                "Bash".to_string(),
                Some("rm -rf /".to_string()),
            )
            .await;

        ch.resolve_decision(
            Some(&db),
            request_id,
            ClassifierVerdict::Denied,
            OperatorDecision::Approve,
            "Bash",
            Some("rm -rf /"),
            DecisionAuthority::Strict,
            DecisionScope::global(),
        )
        .await
        .expect("resolve OK");
        let _ = rx.await;

        // Wait briefly for the spawned DB write to land.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (override_used, authority): (i64, String) = db
            .with_db(move |db| {
                Ok(db.conn.query_row(
                    "SELECT override_used, decision_authority
                         FROM permission_decisions WHERE request_id = ?1",
                    rusqlite::params![request_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .await
            .expect("row present");
        assert_eq!(override_used, 0, "strict authority never flips");
        assert_eq!(authority, "strict");
    }

    /// AC4: Override authority + classifier Denied + operator Approve
    /// → `override_used = 1`. Only combination that flips the flag.
    #[tokio::test]
    async fn override_authority_denied_approve_records_override_used_true() {
        let db = scratch_db().await;
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Denied,
                "Bash".to_string(),
                Some("gh pr merge".to_string()),
            )
            .await;

        ch.resolve_decision(
            Some(&db),
            request_id,
            ClassifierVerdict::Denied,
            OperatorDecision::Approve,
            "Bash",
            Some("gh pr merge"),
            DecisionAuthority::Override,
            DecisionScope::global(),
        )
        .await
        .expect("resolve OK");
        let _ = rx.await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (override_used, authority): (i64, String) = db
            .with_db(move |db| {
                Ok(db.conn.query_row(
                    "SELECT override_used, decision_authority
                         FROM permission_decisions WHERE request_id = ?1",
                    rusqlite::params![request_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .await
            .expect("row present");
        assert_eq!(override_used, 1, "override + denied + approve flips");
        assert_eq!(authority, "override");
    }

    /// AC4: Override authority + classifier Approved + operator Approve
    /// → `override_used = 0` (nothing to flip; classifier already approved).
    #[tokio::test]
    async fn override_authority_approved_approve_records_override_used_false() {
        let db = scratch_db().await;
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Approved,
                "Read".to_string(),
                None,
            )
            .await;

        ch.resolve_decision(
            Some(&db),
            request_id,
            ClassifierVerdict::Approved,
            OperatorDecision::Approve,
            "Read",
            None,
            DecisionAuthority::Override,
            DecisionScope::global(),
        )
        .await
        .expect("resolve OK");
        let _ = rx.await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let override_used: i64 = db
            .with_db(move |db| {
                Ok(db.conn.query_row(
                    "SELECT override_used FROM permission_decisions WHERE request_id = ?1",
                    rusqlite::params![request_id.to_string()],
                    |row| row.get(0),
                )?)
            })
            .await
            .expect("row present");
        assert_eq!(
            override_used, 0,
            "override authority does not flip a non-Deny verdict"
        );
    }

    /// AC4 ordering: the classifier oneshot fires before the DB write
    /// completes — the caller does not observe the DB write as a
    /// precondition.
    #[tokio::test]
    async fn oneshot_fires_before_db_write_lands() {
        let db = scratch_db().await;
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Held,
                "Bash".to_string(),
                Some("ls".to_string()),
            )
            .await;

        let started = std::time::Instant::now();
        ch.resolve_decision(
            Some(&db),
            request_id,
            ClassifierVerdict::Held,
            OperatorDecision::Approve,
            "Bash",
            Some("ls"),
            DecisionAuthority::Strict,
            DecisionScope::global(),
        )
        .await
        .expect("resolve OK");
        let decision = rx.await.expect("receiver");
        let oneshot_elapsed = started.elapsed();

        assert_eq!(decision, OperatorDecision::Approve);
        // Well under the ≤500ms design budget for the oneshot path.
        assert!(
            oneshot_elapsed.as_millis() < 200,
            "oneshot delivery took {oneshot_elapsed:?}, expected <200ms"
        );

        // Confirm the DB write eventually lands.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let count: i64 = db
            .with_db(move |db| {
                Ok(db.conn.query_row(
                    "SELECT COUNT(*) FROM permission_decisions WHERE request_id = ?1",
                    rusqlite::params![request_id.to_string()],
                    |row| row.get(0),
                )?)
            })
            .await
            .expect("query OK");
        assert_eq!(count, 1);
    }

    /// AC6: peek_pending returns snapshot but does NOT consume the entry.
    #[tokio::test]
    async fn peek_pending_returns_snapshot_without_consuming() {
        let ch = PermissionsChannel::new();
        let request_id = Uuid::new_v4();
        let _rx = ch
            .register_pending(
                request_id,
                ClassifierVerdict::Denied,
                "Bash".to_string(),
                Some("dangerous".to_string()),
            )
            .await;

        let snapshot = ch
            .peek_pending(request_id)
            .await
            .expect("peek returns snapshot");
        assert_eq!(snapshot.classifier_verdict, ClassifierVerdict::Denied);
        assert_eq!(snapshot.tool_name, "Bash");
        assert_eq!(snapshot.args_summary.as_deref(), Some("dangerous"));

        // Second peek still succeeds — nothing was consumed.
        let second = ch.peek_pending(request_id).await;
        assert!(second.is_some());

        // Sanity: peek for unknown request_id → None.
        let none = ch.peek_pending(Uuid::new_v4()).await;
        assert!(none.is_none());
    }

    // ── AskUserQuestion (mika#1734) tests ────────────────────────────────

    fn sample_questions() -> Vec<AskQuestion> {
        vec![
            AskQuestion {
                question: "Which coffee?".to_string(),
                options: vec![
                    AskOption {
                        label: "Espresso".to_string(),
                        description: "Strong shot".to_string(),
                    },
                    AskOption {
                        label: "Latte".to_string(),
                        description: "Milky".to_string(),
                    },
                ],
                multi_select: false,
            },
            AskQuestion {
                question: "Size?".to_string(),
                options: vec![
                    AskOption {
                        label: "Small".to_string(),
                        description: "".to_string(),
                    },
                    AskOption {
                        label: "Large".to_string(),
                        description: "".to_string(),
                    },
                ],
                multi_select: false,
            },
        ]
    }

    /// AC1 wire: `AskUserQuestion` frame with structured questions serde
    /// round-trips through the same discriminated union as the permission
    /// request. `multiSelect` camelCases on the wire.
    #[test]
    fn ask_user_question_frame_serde_round_trip() {
        let request_id = Uuid::new_v4();
        let frame = PermissionStreamFrame::AskUserQuestion {
            request_id,
            questions: sample_questions(),
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"event\":\"ask_user_question\""));
        assert!(
            json.contains("\"multiSelect\":false"),
            "expected camelCase multiSelect, got {json}"
        );
        let back: PermissionStreamFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            json,
            "round-trip is stable"
        );
    }

    /// AC1 reuse: `PermissionRequest` and `AskUserQuestion` co-exist on
    /// the same broadcast — a single subscriber receives both.
    #[tokio::test]
    async fn permission_request_and_ask_share_channel() {
        let ch = PermissionsChannel::new();
        let mut rx = ch.outgoing.subscribe();

        // Emit a permission request first.
        let perm_req = Uuid::new_v4();
        ch.broadcast_frame(PermissionStreamFrame::PermissionRequest {
            request_id: perm_req,
            tool_name: "Bash".to_string(),
            args_summary: "ls".to_string(),
            classifier_verdict: ClassifierVerdict::Held,
            held_reason: "needs op".to_string(),
        });

        // Now emit an AskUserQuestion.
        let ask_req = Uuid::new_v4();
        ch.broadcast_frame(PermissionStreamFrame::AskUserQuestion {
            request_id: ask_req,
            questions: sample_questions(),
        });

        let first = rx.recv().await.expect("first frame");
        let second = rx.recv().await.expect("second frame");
        assert!(
            matches!(first, PermissionStreamFrame::PermissionRequest { request_id, .. } if request_id == perm_req)
        );
        assert!(
            matches!(second, PermissionStreamFrame::AskUserQuestion { request_id, .. } if request_id == ask_req)
        );
    }

    /// AC2: happy-path answer routes to classifier.
    #[tokio::test]
    async fn resolve_answer_delivers_answers_to_classifier() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        // Long timeout so it doesn't materialize during the test.
        let rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_secs(30))
            .await;

        let mut answers = HashMap::new();
        answers.insert("0".to_string(), "Espresso".to_string());
        answers.insert("1".to_string(), "Large".to_string());
        ch.resolve_answer(request_id, answers.clone())
            .await
            .unwrap();

        let outcome = rx.await.expect("classifier receives");
        match outcome {
            AnswerResult::Answered { answers: got } => {
                assert_eq!(got, answers);
            }
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    /// AC2: missing answer → MissingAnswer, entry stays pending so a
    /// corrected retry can succeed.
    #[tokio::test]
    async fn resolve_answer_rejects_missing_key() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        let _rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_secs(30))
            .await;

        let mut answers = HashMap::new();
        answers.insert("0".to_string(), "Espresso".to_string());
        // question 1 missing
        let err = ch
            .resolve_answer(request_id, answers)
            .await
            .expect_err("should reject");
        assert!(matches!(
            err,
            AnswerError::MissingAnswer { question_index: 1 }
        ));

        // Retry with the missing answer added succeeds.
        let mut retry = HashMap::new();
        retry.insert("0".to_string(), "Espresso".to_string());
        retry.insert("1".to_string(), "Small".to_string());
        ch.resolve_answer(request_id, retry).await.unwrap();
    }

    /// AC2: invalid option label → InvalidOptionLabel (400).
    #[tokio::test]
    async fn resolve_answer_rejects_invalid_option_label() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        let _rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_secs(30))
            .await;

        let mut answers = HashMap::new();
        answers.insert("0".to_string(), "Cappuccino".to_string());
        answers.insert("1".to_string(), "Small".to_string());
        let err = ch
            .resolve_answer(request_id, answers)
            .await
            .expect_err("should reject");
        assert!(matches!(
            err,
            AnswerError::InvalidOptionLabel {
                question_index: 0,
                ..
            }
        ));
    }

    /// AC2: extra keys (unparseable or out-of-range index) → ExtraKey.
    #[tokio::test]
    async fn resolve_answer_rejects_extra_keys() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        let _rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_secs(30))
            .await;

        let mut answers = HashMap::new();
        answers.insert("0".to_string(), "Espresso".to_string());
        answers.insert("1".to_string(), "Small".to_string());
        answers.insert("nonsense".to_string(), "junk".to_string());
        let err = ch
            .resolve_answer(request_id, answers)
            .await
            .expect_err("should reject");
        assert!(matches!(err, AnswerError::ExtraKey { .. }));
    }

    /// AC2: unknown request_id → UnknownRequest.
    #[tokio::test]
    async fn resolve_answer_unknown_request_id_returns_not_found() {
        let ch = Arc::new(PermissionsChannel::new());
        let mut answers = HashMap::new();
        answers.insert("0".to_string(), "Espresso".to_string());
        let err = ch
            .resolve_answer(Uuid::new_v4(), answers)
            .await
            .expect_err("should reject");
        assert!(matches!(err, AnswerError::UnknownRequest));
    }

    /// AC3: server-side hold-timeout fires `AnswerResult::Timeout` on the
    /// classifier's oneshot when no answer arrives before the deadline.
    #[tokio::test]
    async fn hold_timeout_materializes_operator_timeout() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        let rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_millis(50))
            .await;

        let outcome = rx.await.expect("classifier receives");
        match outcome {
            AnswerResult::Timeout { reason } => {
                assert_eq!(reason, "operator-timeout");
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    /// Peek returns cloned snapshot without removing.
    #[tokio::test]
    async fn peek_pending_ask_returns_snapshot_without_consuming() {
        let ch = Arc::new(PermissionsChannel::new());
        let request_id = Uuid::new_v4();
        let _rx = ch
            .register_pending_ask(request_id, sample_questions(), Duration::from_secs(30))
            .await;

        let snap = ch.peek_pending_ask(request_id).await.expect("present");
        assert_eq!(snap.len(), 2);
        let snap2 = ch
            .peek_pending_ask(request_id)
            .await
            .expect("still present");
        assert_eq!(snap2, snap);

        let none = ch.peek_pending_ask(Uuid::new_v4()).await;
        assert!(none.is_none());
    }

    /// AC2 wire: `deny_unknown_fields` on the answer body rejects extras.
    #[test]
    fn answer_request_rejects_unknown_field() {
        let body = r#"{"answers": {"0": "Espresso"}, "extra": "no"}"#;
        let err = serde_json::from_str::<PermissionAnswerRequest>(body).unwrap_err();
        assert!(err.to_string().contains("extra"));
    }

    /// AC2 wire: minimal happy-path body parses.
    #[test]
    fn answer_request_minimal_body_parses() {
        let body = r#"{"answers": {"0": "Espresso"}}"#;
        let parsed: PermissionAnswerRequest = serde_json::from_str(body).unwrap();
        assert_eq!(
            parsed.answers.get("0").map(|s| s.as_str()),
            Some("Espresso")
        );
    }
}
