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
use tracing::{debug, error, info};

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

/// Max kinds accepted by POST. Mirrors the migration CHECK constraint;
/// validating in Rust returns a 400 with a useful error before the DB rejects
/// with a generic constraint violation.
const VALID_KINDS: &[&str] = &["handoff", "update", "ack"];

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

    if !is_valid_orchestrator_id(&orchestrator_id) {
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

    if !is_valid_orchestrator_id(&orchestrator_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid orchestrator_id: must be 1-128 chars, [A-Za-z0-9_-]"
            })),
        )
            .into_response();
    }

    let cursor = parse_last_event_id(&headers).unwrap_or(0);
    info!(
        event = "orchestrator_inbox_subscriber_connected",
        orchestrator_id = %orchestrator_id,
        cursor,
        "orchestrator inbox SSE subscriber connected"
    );

    let stream = inbox_stream(state.pool.clone(), orchestrator_id, cursor);
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(ORCHESTRATOR_INBOX_KEEPALIVE_INTERVAL)
                .text("keep-alive"),
        )
        .into_response()
}

/// Build the SSE event stream. Long-poll loop reads rows by cursor, then sleeps
/// `ORCHESTRATOR_INBOX_POLL_INTERVAL` between polls. On row read, marks the row
/// `delivered_at` (observational; replay-from-cursor is authoritative).
fn inbox_stream(
    pool: PgPool,
    orchestrator_id: String,
    initial_cursor: i64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
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
                        mark_delivered(&pool, event_id).await;
                        yield Ok(Event::default()
                            .id(event_id.to_string())
                            .event("message")
                            .data(data));
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

/// Run the retention task forever. Spawned as a tokio task in `main.rs`.
/// Purges rows older than `ORCHESTRATOR_INBOX_RETENTION_DAYS`.
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

/// Validate the `orchestrator_id` URL path segment. Stricter than the bearer
/// auth allows by design — id is not a secret, but it gates path-based
/// scoping and ends up in log lines.
fn is_valid_orchestrator_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse the `Last-Event-Id` header into a cursor value. Missing or malformed
/// → `None` (caller defaults to 0 = full replay from start).
fn parse_last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
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
            telegram,
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: SecretString::from("b".repeat(64)),
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
    fn is_valid_orchestrator_id_accepts_uuid() {
        assert!(is_valid_orchestrator_id(
            "550e8400-e29b-41d4-a716-446655440000"
        ));
    }

    #[test]
    fn is_valid_orchestrator_id_accepts_timestamp_pid_fallback() {
        // matches the `scripts/mika-orchestrator-id` fallback shape
        assert!(is_valid_orchestrator_id("20260517T204027Z-12345"));
    }

    #[test]
    fn is_valid_orchestrator_id_accepts_alphanumeric_with_underscore() {
        assert!(is_valid_orchestrator_id("vincent_desktop_orch_1"));
    }

    #[test]
    fn is_valid_orchestrator_id_rejects_empty() {
        assert!(!is_valid_orchestrator_id(""));
    }

    #[test]
    fn is_valid_orchestrator_id_rejects_path_traversal() {
        assert!(!is_valid_orchestrator_id("../etc/passwd"));
        assert!(!is_valid_orchestrator_id("foo/bar"));
    }

    #[test]
    fn is_valid_orchestrator_id_rejects_special_chars() {
        assert!(!is_valid_orchestrator_id("foo bar"));
        assert!(!is_valid_orchestrator_id("foo@bar"));
        assert!(!is_valid_orchestrator_id("foo$bar"));
    }

    #[test]
    fn is_valid_orchestrator_id_rejects_overlong() {
        let long_id = "a".repeat(129);
        assert!(!is_valid_orchestrator_id(&long_id));
        let max_id = "a".repeat(128);
        assert!(is_valid_orchestrator_id(&max_id));
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
    fn valid_kinds_match_check_constraint() {
        // Migration 007 has CHECK (kind IN ('handoff','update','ack')) — list must match
        assert_eq!(VALID_KINDS, &["handoff", "update", "ack"]);
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
