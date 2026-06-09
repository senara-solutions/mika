//! Orchestrator inbox: real-time spawn-to-orchestrator coordination channel.
//!
//! Spawned Claude Code tenants POST handoff messages addressed to their parent
//! orchestrator's id; the orchestrator subscribes via SSE. Messages are
//! persisted in Postgres (`orchestrator_inbox_messages`, migration 007) so a
//! gateway pod restart causes a reconnect gap, not data loss — SSE clients
//! replay from `Last-Event-Id` cursor.
//!
//! See mika#1189 plan:
//! `docs/plans/2026-05-17-003-feat-1189-mika-gateway-orchestrator-inbox-v2-plan.md`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

use crate::routes::AppState;

// -- Public constants (tunable via code edit, not env — see plan NF3) --

/// How often the SSE handler polls Postgres for new rows. Plan calibration:
/// 1.5 s matches the "5+ paste-cycles → real-time" UX target (sub-2-s push)
/// without thrashing the database. Tune via code edit if volume changes.
pub const ORCHESTRATOR_INBOX_POLL_INTERVAL: Duration = Duration::from_millis(1500);

/// Retention window for orchestrator inbox rows. Mirrors the 7-day window used
/// by `outbound_messages` (see `crates/mika-gateway/CLAUDE.md` → "Agent
/// Identification & Reply Routing"). Operator-controlled developer
/// infrastructure — short enough to bound table growth, long enough to survive
/// a long-weekend orchestrator outage.
pub const ORCHESTRATOR_INBOX_RETENTION_DAYS: i64 = 7;

/// SSE keep-alive ping cadence. 30 s matches the plan's contract for clients
/// detecting dead connections (proxy/LB idle timeouts typically 60 s+).
const ORCHESTRATOR_INBOX_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// How often the retention task wakes to purge stale rows.
const RETENTION_TICK_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Max kinds accepted by POST. Mirrors the migration 007 CHECK constraint;
/// the `valid_kinds_match_check_constraint` test parses the migration SQL to
/// catch drift between this slice and the SQL constraint.
const VALID_KINDS: &[&str] = &["handoff", "update", "ack"];

/// Default cap on concurrent SSE subscribers. The polling loop runs one
/// Postgres `fetch_after_cursor` per `ORCHESTRATOR_INBOX_POLL_INTERVAL` per
/// subscriber, so unbounded subscribers can saturate the gateway pool (capped
/// at 20 connections in `main.rs`). 10 permits keeps DB pressure well under
/// the pool budget while leaving headroom for the DLQ worker and Telegram
/// webhooks, which share the same pool. Tune in `main.rs` by changing the
/// `Semaphore::new(...)` argument; this constant is only the default callers
/// receive via `default_inbox_subscriber_semaphore`.
pub const ORCHESTRATOR_INBOX_DEFAULT_SUBSCRIBER_CAP: usize = 10;

/// `Retry-After` value advertised to SSE clients that hit the subscriber cap.
/// Short enough to reconnect promptly after another subscriber disconnects;
/// long enough to avoid thundering-herd retries on a sustained over-cap.
const SUBSCRIBER_CAP_RETRY_AFTER_SECS: u64 = 5;

/// Cursor value above this in a `Last-Event-Id` header is suspicious — no real
/// BIGSERIAL on this table will ever reach it. We don't reject (the SSE spec
/// has no defined behavior here and the client may have made a typo), but we
/// log a warning so operators see the empty-poll loop's likely cause.
const SUSPICIOUS_CURSOR_THRESHOLD: i64 = 1_000_000_000_000_000;

/// Convenience constructor for the SSE subscriber semaphore. Callers that
/// want a different cap should `Arc::new(Semaphore::new(N))` themselves.
pub fn default_inbox_subscriber_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(ORCHESTRATOR_INBOX_DEFAULT_SUBSCRIBER_CAP))
}

// -- Types --

/// Body of `POST /orchestrator/inbox/{orchestrator_id}/message`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PostMessagePayload {
    /// Spawn correlation id (null for orchestrator-originated messages — reserved for v2+).
    #[serde(default)]
    pub spawn_id: Option<String>,
    /// Message kind: `handoff`, `update`, or `ack`.
    #[schema(example = "handoff")]
    pub kind: String,
    /// Free-form JSON body. For `handoff`, mirrors the mika-platform#100
    /// filesystem-inbox entry shape.
    pub body: serde_json::Value,
}

/// Response from POST: persisted row id (monotonic, used as SSE cursor).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PostMessageResponse {
    #[schema(example = 42)]
    pub id: i64,
}

/// One row from `orchestrator_inbox_messages`, surfaced as SSE event data.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub(crate) struct InboxMessage {
    pub id: i64,
    pub orchestrator_id: String,
    pub spawn_id: Option<String>,
    pub kind: String,
    pub body: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

// -- POST handler --

/// `POST /orchestrator/inbox/{orchestrator_id}/message`.
///
/// Bearer auth applied by `require_bearer_token` middleware. 256 KB body limit
/// applied at the route layer.
#[utoipa::path(
    post,
    path = "/orchestrator/inbox/{orchestrator_id}/message",
    params(
        ("orchestrator_id" = String, Path,
         description = "Orchestrator session id, 1-128 chars [A-Za-z0-9_-]")
    ),
    request_body = PostMessagePayload,
    responses(
        (status = 201, description = "Message persisted", body = PostMessageResponse),
        (status = 400, description = "Invalid orchestrator_id, malformed body, or unknown kind"),
        (status = 401, description = "Missing or invalid Bearer token"),
        (status = 404, description = "Endpoint disabled (MIKA_ORCHESTRATOR_INBOX_ENABLED unset or 0)"),
        (status = 413, description = "Body exceeds 256KB"),
        (status = 500, description = "Database write failed"),
    ),
    security(("bearer" = []))
)]
pub(crate) async fn handle_post_message(
    State(state): State<AppState>,
    Path(orchestrator_id): Path<String>,
    Json(payload): Json<PostMessagePayload>,
) -> Response {
    if !state.orchestrator_inbox_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !is_valid_id_token(&orchestrator_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid orchestrator_id: must be 1-128 chars, [A-Za-z0-9_-]"
            })),
        )
            .into_response();
    }

    if !VALID_KINDS.contains(&payload.kind.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("invalid kind: must be one of {VALID_KINDS:?}")
            })),
        )
            .into_response();
    }

    if let Some(ref spawn_id) = payload.spawn_id
        && !is_valid_id_token(spawn_id)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid spawn_id: must be 1-128 chars, [A-Za-z0-9_-]"
            })),
        )
            .into_response();
    }

    match insert_message(&state.pool, &orchestrator_id, &payload).await {
        Ok(id) => {
            info!(
                event = "orchestrator_inbox_message_received",
                orchestrator_id = %orchestrator_id,
                spawn_id = ?payload.spawn_id,
                kind = %payload.kind,
                id,
                "orchestrator inbox message persisted"
            );
            (StatusCode::CREATED, Json(PostMessageResponse { id })).into_response()
        }
        Err(e) => {
            error!(
                error = %e,
                orchestrator_id = %orchestrator_id,
                "orchestrator inbox insert failed"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn insert_message(
    pool: &PgPool,
    orchestrator_id: &str,
    payload: &PostMessagePayload,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO orchestrator_inbox_messages (orchestrator_id, spawn_id, kind, body)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(orchestrator_id)
    .bind(payload.spawn_id.as_deref())
    .bind(&payload.kind)
    .bind(&payload.body)
    .fetch_one(pool)
    .await
}

// -- SSE GET handler --

/// `GET /orchestrator/inbox/{orchestrator_id}/stream`.
///
/// Returns `text/event-stream`. Each event:
/// ```text
/// id: <bigserial>
/// event: message
/// data: {"id":<n>,"orchestrator_id":"...","spawn_id":"...","kind":"...","body":{...},"created_at":"..."}
/// ```
///
/// Replay-from-cursor: client sends `Last-Event-Id: <last-seen-id>` and the
/// server emits all rows with `id > last-seen-id`. After replay, polls every
/// `ORCHESTRATOR_INBOX_POLL_INTERVAL` for new rows. `KeepAlive` emits a comment
/// ping every `ORCHESTRATOR_INBOX_KEEPALIVE_INTERVAL`.
#[utoipa::path(
    get,
    path = "/orchestrator/inbox/{orchestrator_id}/stream",
    params(
        ("orchestrator_id" = String, Path,
         description = "Orchestrator session id, 1-128 chars [A-Za-z0-9_-]"),
        ("last-event-id" = Option<i64>, Header,
         description = "Resume cursor — server emits rows with id > this value")
    ),
    responses(
        (status = 200, description = "text/event-stream with replay + live tail"),
        (status = 400, description = "Invalid orchestrator_id"),
        (status = 401, description = "Missing or invalid Bearer token"),
        (status = 404, description = "Endpoint disabled (MIKA_ORCHESTRATOR_INBOX_ENABLED unset or 0)"),
    ),
    security(("bearer" = []))
)]
pub(crate) async fn handle_stream(
    State(state): State<AppState>,
    Path(orchestrator_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !state.orchestrator_inbox_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    if !is_valid_id_token(&orchestrator_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid orchestrator_id: must be 1-128 chars, [A-Za-z0-9_-]"
            })),
        )
            .into_response();
    }

    // SSE subscriber cap (mika#1189 review ADV-001/SEC-001). Each subscriber
    // runs an independent Postgres-polling loop; without a cap a misbehaving
    // caller can saturate the shared connection pool and cascade into
    // webhook-delivery failures. Permit is held for the lifetime of the
    // stream via the async_stream closure below.
    let permit = match state.inbox_subscriber_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!(
                event = "orchestrator_inbox_subscriber_capped",
                orchestrator_id = %orchestrator_id,
                "orchestrator inbox SSE subscriber rejected — at capacity"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", SUBSCRIBER_CAP_RETRY_AFTER_SECS.to_string())],
                Json(serde_json::json!({
                    "error": "orchestrator inbox at subscriber capacity; retry later"
                })),
            )
                .into_response();
        }
    };

    let cursor = parse_last_event_id(&headers).unwrap_or(0);
    info!(
        event = "orchestrator_inbox_subscriber_connected",
        orchestrator_id = %orchestrator_id,
        cursor,
        "orchestrator inbox SSE subscriber connected"
    );

    let stream = inbox_stream(state.pool.clone(), orchestrator_id, cursor, permit);
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(ORCHESTRATOR_INBOX_KEEPALIVE_INTERVAL)
                .text("keep-alive"),
        )
        .into_response()
}

/// Build the SSE event stream. Long-poll loop reads rows by cursor, then sleeps
/// `ORCHESTRATOR_INBOX_POLL_INTERVAL` between polls. `mark_delivered` runs
/// *after* `yield` so a client that disconnects between cursor advance and
/// row emission doesn't get the row silently marked delivered — cursor
/// replay from `Last-Event-Id` is the authoritative redelivery channel and
/// `delivered_at` stays a best-effort observability signal.
///
/// `_permit` is the subscriber-cap permit acquired by the caller; we hold it
/// for the stream's lifetime so it's released on disconnect (drop) and the
/// next subscriber can connect.
fn inbox_stream(
    pool: PgPool,
    orchestrator_id: String,
    initial_cursor: i64,
    _permit: tokio::sync::OwnedSemaphorePermit,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // Move the permit into the stream closure so it stays alive until
        // the consumer drops the response. Referenced via `let _ = ...` so
        // the compiler doesn't optimize the drop point earlier.
        let _hold = _permit;
        let mut cursor = initial_cursor;
        loop {
            match fetch_after_cursor(&pool, &orchestrator_id, cursor).await {
                Ok(rows) => {
                    for row in rows {
                        let event_id = row.id;
                        let data = match serde_json::to_string(&row) {
                            Ok(s) => s,
                            Err(e) => {
                                error!(error = %e, id = row.id, "orchestrator inbox row serialize failed");
                                continue;
                            }
                        };
                        cursor = event_id;
                        yield Ok(Event::default()
                            .id(event_id.to_string())
                            .event("message")
                            .data(data));
                        // mark_delivered runs AFTER yield (review COR-02).
                        // If the client disconnected mid-yield, the row is
                        // never marked — next reconnect's cursor replay
                        // picks it up. `delivered_at` is therefore a true
                        // observability signal of "subscriber saw the row
                        // and the gateway is still alive."
                        mark_delivered(&pool, event_id).await;
                    }
                }
                Err(e) => {
                    error!(error = %e, orchestrator_id = %orchestrator_id, "orchestrator inbox poll query failed");
                }
            }
            tokio::time::sleep(ORCHESTRATOR_INBOX_POLL_INTERVAL).await;
        }
    }
}

async fn fetch_after_cursor(
    pool: &PgPool,
    orchestrator_id: &str,
    cursor: i64,
) -> Result<Vec<InboxMessage>, sqlx::Error> {
    sqlx::query_as::<_, InboxMessage>(
        r#"
        SELECT id, orchestrator_id, spawn_id, kind, body, created_at
        FROM orchestrator_inbox_messages
        WHERE orchestrator_id = $1 AND id > $2
        ORDER BY id ASC
        LIMIT 500
        "#,
    )
    .bind(orchestrator_id)
    .bind(cursor)
    .fetch_all(pool)
    .await
}

/// Mark a row as delivered (observational — does NOT gate redelivery; cursor
/// replay is authoritative). Best-effort; logged on error but does not abort.
async fn mark_delivered(pool: &PgPool, id: i64) {
    if let Err(e) = sqlx::query(
        "UPDATE orchestrator_inbox_messages SET delivered_at = now() WHERE id = $1 AND delivered_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await
    {
        debug!(error = %e, id, "orchestrator inbox mark_delivered failed");
    }
}

// -- Retention task --

/// Run the retention task forever. Spawned as a tokio task in `main.rs` only
/// when `MIKA_ORCHESTRATOR_INBOX_ENABLED=1` — when the feature is off, the
/// migration may not be in place and the DELETE would error every hour.
/// Purges rows older than `ORCHESTRATOR_INBOX_RETENTION_DAYS`.
///
// TODO(mika#1189-followup): integration test against a real Postgres
// instance for `purge_old_rows` semantics (boundary at the 7-day cutoff,
// no-op when zero rows match). Skipped here because the gateway test
// harness uses lazy pool connect_lazy() which would 1s-timeout on a real
// DELETE. Needs a docker-postgres or sqlx::test! harness — out of scope
// for this PR.
pub async fn run_retention_task(pool: PgPool) {
    info!(
        retention_days = ORCHESTRATOR_INBOX_RETENTION_DAYS,
        "orchestrator inbox retention task started"
    );
    loop {
        tokio::time::sleep(RETENTION_TICK_INTERVAL).await;
        match purge_old_rows(&pool).await {
            Ok(0) => debug!("orchestrator inbox retention sweep: nothing to purge"),
            Ok(n) => info!(
                event = "orchestrator_inbox_retention_purged",
                rows = n,
                retention_days = ORCHESTRATOR_INBOX_RETENTION_DAYS,
                "orchestrator inbox retention sweep purged rows"
            ),
            Err(e) => error!(error = %e, "orchestrator inbox retention sweep failed"),
        }
    }
}

async fn purge_old_rows(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM orchestrator_inbox_messages WHERE created_at < now() - ($1 || ' days')::interval",
    )
    .bind(ORCHESTRATOR_INBOX_RETENTION_DAYS.to_string())
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// -- Helpers --

/// Validate a path-segment id (orchestrator_id, spawn_id). Stricter than the
/// bearer auth allows by design — these are not secrets, but they gate
/// path-based scoping, end up in log lines, and are emitted in SSE event
/// bodies. Same rule as claude-pilot-py's `_is_valid_id_token`.
fn is_valid_id_token(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse the `Last-Event-Id` header into a cursor value. Missing, malformed,
/// or negative → `None` (caller defaults to 0 = full replay from start).
/// Suspiciously large cursors (above `SUSPICIOUS_CURSOR_THRESHOLD`) parse
/// successfully but emit a WARN: they're almost certainly a client bug or
/// header tampering, and they manifest as a permanent empty-poll loop with
/// keep-alives but no events — easy to miss without a log line.
fn parse_last_event_id(headers: &HeaderMap) -> Option<i64> {
    let parsed = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())?;
    if parsed < 0 {
        debug!(
            cursor = parsed,
            "negative Last-Event-Id treated as full replay"
        );
        return None;
    }
    if parsed > SUSPICIOUS_CURSOR_THRESHOLD {
        warn!(
            cursor = parsed,
            "Last-Event-Id cursor exceeds SUSPICIOUS_CURSOR_THRESHOLD; subscriber will see only keep-alives until cursor catches up or client reconnects"
        );
    }
    Some(parsed)
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use http_body_util::BodyExt;
    use secrecy::SecretString;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use crate::routes::{AppState, build_router};

    /// Build a test AppState with a lazy Postgres pool that never actually
    /// connects — handlers that don't reach the DB (auth rejection,
    /// feature-flag gate, validation) run cleanly; handlers that DO reach the
    /// DB will hit a connect timeout, surfaceable as 500.
    fn test_state(orchestrator_inbox_enabled: bool) -> AppState {
        test_state_with_subscriber_cap(orchestrator_inbox_enabled, 10)
    }

    fn test_state_with_subscriber_cap(
        orchestrator_inbox_enabled: bool,
        subscriber_cap: usize,
    ) -> AppState {
        let http_client = reqwest::Client::new();
        let telegram = crate::telegram::TelegramClient::new(
            http_client.clone(),
            SecretString::from("fake-bot-token"),
        );
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        AppState {
            pool,
            telegram: Some(telegram),
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: Some(SecretString::from("b".repeat(64))),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: None,
            agents_namespace: "mika-agents".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: None,
            github_delivery_cache: crate::github::new_delivery_cache(),
            github_app: None,
            github_api_base_url: None,
            orchestrator_inbox_enabled,
            inbox_subscriber_semaphore: Arc::new(Semaphore::new(subscriber_cap)),
        }
    }

    fn build_test_router(enabled: bool) -> Router {
        build_router(test_state(enabled))
    }

    async fn body_to_string(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    // -- Auth tests --

    #[tokio::test]
    async fn post_missing_bearer_returns_401() {
        let app = build_test_router(true);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/test-orch/message")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"kind":"handoff","body":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn post_wrong_bearer_returns_401() {
        let app = build_test_router(true);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/test-orch/message")
                    .header("authorization", "Bearer wrong-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"kind":"handoff","body":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stream_missing_bearer_returns_401() {
        let app = build_test_router(true);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/orchestrator/inbox/test-orch/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // -- Feature-flag tests (with valid auth) --

    #[tokio::test]
    async fn post_returns_404_when_feature_disabled() {
        let app = build_test_router(false);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/test-orch/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"kind":"handoff","body":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "POST must return 404 when MIKA_ORCHESTRATOR_INBOX_ENABLED is off"
        );
    }

    #[tokio::test]
    async fn stream_returns_404_when_feature_disabled() {
        let app = build_test_router(false);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/orchestrator/inbox/test-orch/stream")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "stream must return 404 when MIKA_ORCHESTRATOR_INBOX_ENABLED is off"
        );
    }

    // -- Validation tests (enabled + valid auth) --

    #[tokio::test]
    async fn post_rejects_invalid_orchestrator_id() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/../etc/passwd/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"kind":"handoff","body":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Path-traversal attempts don't match the route pattern → 404 from axum
        // OR are normalized → 400 from our handler. Either is a rejection.
        assert!(
            resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
            "expected rejection for traversal id, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn post_rejects_invalid_kind() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/valid-id-123/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"kind":"invalid_kind","body":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_to_string(resp.into_body()).await;
        assert!(body.contains("invalid kind"), "body: {body}");
    }

    #[tokio::test]
    async fn post_rejects_malformed_json() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/valid-id-123/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{not valid json"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Axum returns 400 for malformed JSON when extracting Json<T>
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_rejects_invalid_spawn_id() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        // 200 chars exceeds the 128-char id limit
        let bad_spawn = "a".repeat(200);
        let payload = serde_json::json!({
            "spawn_id": bad_spawn,
            "kind": "handoff",
            "body": {}
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/valid-id-123/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_to_string(resp.into_body()).await;
        assert!(body.contains("spawn_id"), "body: {body}");
    }

    #[tokio::test]
    async fn post_rejects_spawn_id_with_special_chars() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        let payload = serde_json::json!({
            "spawn_id": "spawn\nwith\nnewlines",
            "kind": "handoff",
            "body": {}
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/valid-id-123/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stream_returns_503_when_subscriber_cap_full() {
        // Build state with 1-permit semaphore, hold the permit, then attempt
        // to open a second SSE stream — must get 503 + Retry-After.
        let state = test_state_with_subscriber_cap(true, 1);
        let held = state
            .inbox_subscriber_semaphore
            .clone()
            .try_acquire_owned()
            .expect("initial permit available");
        let app = crate::routes::build_router(state);
        let token = "a".repeat(64);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/orchestrator/inbox/orch-1/stream")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            Some(SUBSCRIBER_CAP_RETRY_AFTER_SECS.to_string().as_str())
        );
        drop(held);
    }

    #[tokio::test]
    async fn post_oversized_body_rejected() {
        let app = build_test_router(true);
        let token = "a".repeat(64);
        // RequestBodyLimitLayer 256KB; this 300KB body must reject.
        let huge = "x".repeat(300 * 1024);
        let payload = serde_json::json!({"kind":"handoff","body":{"data": huge}}).to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/orchestrator/inbox/valid-id-123/message")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        // tower-http's RequestBodyLimitLayer returns 413 Payload Too Large
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn constants_match_plan() {
        assert_eq!(
            ORCHESTRATOR_INBOX_POLL_INTERVAL,
            Duration::from_millis(1500),
            "poll interval must match plan PAC7"
        );
        assert_eq!(
            ORCHESTRATOR_INBOX_RETENTION_DAYS, 7,
            "retention must match plan PAC7"
        );
    }

    #[test]
    fn is_valid_id_token_accepts_uuid() {
        assert!(is_valid_id_token("550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn is_valid_id_token_accepts_timestamp_pid_fallback() {
        // matches the `scripts/mika-orchestrator-id` fallback shape
        assert!(is_valid_id_token("20260517T204027Z-12345"));
    }

    #[test]
    fn is_valid_id_token_accepts_alphanumeric_with_underscore() {
        assert!(is_valid_id_token("vincent_desktop_orch_1"));
    }

    #[test]
    fn is_valid_id_token_rejects_empty() {
        assert!(!is_valid_id_token(""));
    }

    #[test]
    fn is_valid_id_token_rejects_path_traversal() {
        assert!(!is_valid_id_token("../etc/passwd"));
        assert!(!is_valid_id_token("foo/bar"));
    }

    #[test]
    fn is_valid_id_token_rejects_special_chars() {
        assert!(!is_valid_id_token("foo bar"));
        assert!(!is_valid_id_token("foo@bar"));
        assert!(!is_valid_id_token("foo$bar"));
    }

    #[test]
    fn is_valid_id_token_rejects_overlong() {
        let long_id = "a".repeat(129);
        assert!(!is_valid_id_token(&long_id));
        let max_id = "a".repeat(128);
        assert!(is_valid_id_token(&max_id));
    }

    #[test]
    fn parse_last_event_id_extracts_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("42"));
        assert_eq!(parse_last_event_id(&headers), Some(42));
    }

    #[test]
    fn parse_last_event_id_missing_returns_none() {
        let headers = HeaderMap::new();
        assert_eq!(parse_last_event_id(&headers), None);
    }

    #[test]
    fn parse_last_event_id_malformed_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("not-a-number"));
        assert_eq!(parse_last_event_id(&headers), None);
    }

    #[test]
    fn parse_last_event_id_negative_returns_none() {
        // Per review ADV-002: negative cursors are treated as full replay
        // (None → caller defaults to 0). Prevents a client mistake from
        // putting the stream into a partial-replay state.
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("-1"));
        assert_eq!(parse_last_event_id(&headers), None);
    }

    #[test]
    fn parse_last_event_id_suspiciously_large_returns_value() {
        // Cursor above SUSPICIOUS_CURSOR_THRESHOLD parses successfully but
        // emits a warn (we can't easily assert on the log; this test just
        // confirms we still accept the value rather than silently dropping
        // it — that's the documented contract).
        let huge = SUSPICIOUS_CURSOR_THRESHOLD + 1;
        let mut headers = HeaderMap::new();
        headers.insert(
            "last-event-id",
            HeaderValue::from_str(&huge.to_string()).unwrap(),
        );
        assert_eq!(parse_last_event_id(&headers), Some(huge));
    }

    #[test]
    fn parse_last_event_id_zero_returns_zero() {
        // Zero is a legal cursor (it means "no events seen yet, replay all").
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("0"));
        assert_eq!(parse_last_event_id(&headers), Some(0));
    }

    #[test]
    fn post_payload_deserializes_handoff() {
        let json =
            r#"{"spawn_id":"abc-123","kind":"handoff","body":{"status":"success","turns":12}}"#;
        let payload: PostMessagePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.spawn_id.as_deref(), Some("abc-123"));
        assert_eq!(payload.kind, "handoff");
        assert!(payload.body.is_object());
    }

    #[test]
    fn post_payload_deserializes_null_spawn_id() {
        let json = r#"{"spawn_id":null,"kind":"update","body":{"msg":"heartbeat"}}"#;
        let payload: PostMessagePayload = serde_json::from_str(json).unwrap();
        assert!(payload.spawn_id.is_none());
    }

    #[test]
    fn post_payload_deserializes_missing_spawn_id() {
        let json = r#"{"kind":"ack","body":{}}"#;
        let payload: PostMessagePayload = serde_json::from_str(json).unwrap();
        assert!(payload.spawn_id.is_none());
        assert_eq!(payload.kind, "ack");
    }

    #[test]
    fn valid_kinds_match_migration_check_constraint() {
        // The Rust VALID_KINDS slice mirrors the SQL CHECK constraint in
        // migration 007. If either side adds, removes, or renames a kind
        // without updating the other, this test fails — the previous
        // tautological `assert_eq!(VALID_KINDS, &["handoff",...])` could not
        // catch that drift (review finding M1).
        let migration_sql = include_str!("../migrations/007_orchestrator_inbox_messages.sql");
        // Extract the bracketed expression of `kind IN (...)`.
        let in_clause_start = migration_sql
            .find("kind IN (")
            .expect("migration 007 must contain `kind IN (` clause");
        let after = &migration_sql[in_clause_start + "kind IN (".len()..];
        let in_clause_end = after.find(')').expect("kind IN clause must close with )");
        let kinds_list = &after[..in_clause_end];

        // Each VALID_KINDS entry must appear in the SQL list, quoted.
        for kind in VALID_KINDS {
            let needle = format!("'{kind}'");
            assert!(
                kinds_list.contains(&needle),
                "VALID_KINDS contains {kind:?} but migration 007 CHECK constraint does not: {kinds_list}"
            );
        }

        // Conversely: every quoted kind in the SQL list must appear in
        // VALID_KINDS. Catches the case where someone adds 'dispatch' to
        // the migration but forgets to allow it in the handler.
        for raw in kinds_list.split(',') {
            let trimmed = raw.trim().trim_matches('\'');
            if trimmed.is_empty() {
                continue;
            }
            assert!(
                VALID_KINDS.contains(&trimmed),
                "migration 007 CHECK lists {trimmed:?} but VALID_KINDS does not: {VALID_KINDS:?}"
            );
        }
    }

    #[test]
    fn keepalive_smaller_than_typical_proxy_idle_timeout() {
        // 30s ping must be shorter than typical 60s+ LB/proxy idle timeouts so
        // intermediaries don't close idle SSE connections.
        assert!(ORCHESTRATOR_INBOX_KEEPALIVE_INTERVAL < Duration::from_secs(60));
    }

    #[test]
    fn retention_tick_at_least_an_hour() {
        // Avoid hammering the DB with retention sweeps. Plan §4 suggests
        // background tick mirroring outbound_messages; an hour is a safe floor.
        assert!(RETENTION_TICK_INTERVAL >= Duration::from_secs(60 * 60));
    }
}
