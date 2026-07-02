use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{self, HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use secrecy::{ExposeSecret, SecretString};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::a2a_routes;
use crate::github;
use crate::orchestrator_inbox;
use crate::telegram::{
    CustomerTelegramClient, ParsedMessage, TelegramApiError, TelegramClient, TelegramUpdate,
    parse_agent_prefix, parse_update,
};

/// Carries HTTP method and path from request to response extensions,
/// making them available to TraceLayer's `on_response` callback as
/// top-level JSON fields (not nested inside the `spans` array).
#[derive(Clone, Debug)]
struct RequestMeta {
    method: String,
    path: String,
}

/// Middleware that captures method and path from the request and injects
/// them into response extensions for downstream logging.
async fn inject_request_meta(request: Request, next: Next) -> Response {
    let meta = RequestMeta {
        method: request.method().to_string(),
        path: request.uri().path().to_owned(),
    };
    let mut response = next.run(request).await;
    response.extensions_mut().insert(meta);
    response
}

/// Build version info returned by the `/version` endpoint.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct VersionInfo {
    /// Semantic version from Cargo.toml
    #[schema(example = "0.4.0")]
    version: &'static str,
    /// Short git commit hash captured at compile time
    #[schema(example = "abc1234")]
    git_hash: &'static str,
}

/// GET /version — Build version and git hash (no auth).
///
/// Returns build version from Cargo.toml and short git hash captured at compile
/// time. Falls back to "unknown" for git_hash when .git is absent.
#[utoipa::path(
    get,
    path = "/version",
    responses(
        (status = 200, description = "Build version info", body = VersionInfo),
    ),
)]
pub(crate) async fn handle_version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        git_hash: option_env!("GIT_HASH").unwrap_or("unknown"),
    })
}

// -- AppState --

/// Shared application state for the gateway.
/// All fields are Clone-able (owned or Arc-wrapped).
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Global Telegram client — populated when `MIKA_TELEGRAM_BOT_TOKEN` is configured.
    /// Used for outbound delivery via `/send` (operator agents without `customer_id`).
    /// `None` only when no bot token is set.
    pub telegram: Option<TelegramClient>,
    pub http_client: reqwest::Client,
    pub internal_token: SecretString,
    /// Global webhook secret — populated only in single-bot mode. `None` in per-customer mode.
    pub webhook_secret: Option<SecretString>,
    pub ready: Arc<AtomicBool>,
    pub webhook_semaphore: Arc<tokio::sync::Semaphore>,
    /// Optional override for agent container base URL (local E2E testing).
    pub agent_base_url: Option<String>,
    /// Namespace where agent pods run (for FQDN DNS resolution).
    pub agents_namespace: String,
    /// Counter for periodic outbound_messages cleanup (every ~100 webhook calls).
    pub webhook_counter: Arc<AtomicU64>,
    /// Secret for validating inbound GitHub App webhooks (HMAC-SHA256).
    /// When `None`, `POST /webhook/github` returns 404.
    pub github_webhook_secret: Option<SecretString>,
    /// LRU cache for GitHub webhook delivery ID deduplication.
    pub github_delivery_cache: Arc<std::sync::Mutex<lru::LruCache<String, ()>>>,
    /// GitHub App for authenticating outbound GitHub API calls (synchronize no-diff guard).
    /// `None` when credentials are incomplete — all synchronize events pass through (fail-open).
    pub github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
    /// Override for GitHub API base URL (testing only). Defaults to `https://api.github.com`.
    pub github_api_base_url: Option<String>,
    /// Whether the orchestrator inbox endpoints serve requests (mika#1189).
    /// When `false`, both `/orchestrator/inbox/{id}/message` and `.../stream`
    /// return 404 — preserves the pre-1189 filesystem-inbox-only behavior.
    pub orchestrator_inbox_enabled: bool,
    /// Cap on concurrent SSE subscribers for `/orchestrator/inbox/{id}/stream`.
    /// Each subscriber runs an independent Postgres poll loop; the pool is
    /// shared with webhook delivery and the DLQ worker, so unbounded
    /// subscribers can cascade into webhook failures. Default 10 permits
    /// (see `orchestrator_inbox::ORCHESTRATOR_INBOX_DEFAULT_SUBSCRIBER_CAP`).
    pub inbox_subscriber_semaphore: Arc<tokio::sync::Semaphore>,
    /// Public HTTPS base URL of the gateway. Required for per-customer webhook
    /// registration (`POST /admin/customers`). `None` when not configured.
    pub gateway_external_url: Option<String>,
    /// Per-target-agent circuit breaker (mika#1710). Shared 429 health state that
    /// short-circuits webhook deliveries to a saturated agent straight to the DLQ
    /// instead of hammering it with independent per-event retry chains. This is the
    /// missing cross-event coordination layer that let the 2026-07-01 429 flood
    /// self-amplify past the drain rate.
    pub target_health: Arc<crate::circuit_breaker::TargetCircuitBreaker>,
    /// In-flight delivery bound (mika#1710 R4/AC4). Caps concurrently-spawned
    /// delivery tasks at `MAX_INFLIGHT_DELIVERIES`; overflow sheds durably to the
    /// DLQ (drop-oldest via Postgres) rather than accumulating unbounded tasks.
    pub delivery_slots: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("internal_token", &"[REDACTED]")
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("agent_base_url", &self.agent_base_url)
            .field("agents_namespace", &self.agents_namespace)
            .field("webhook_counter", &self.webhook_counter)
            .field(
                "github_webhook_secret",
                &self.github_webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish_non_exhaustive()
    }
}

// -- Router --

/// Build the Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Webhook: Telegram sends updates here (validated by secret_token header)
        .route(
            "/webhook/telegram",
            post(handle_webhook).layer(RequestBodyLimitLayer::new(64 * 1024)),
        )
        // Webhook: Per-customer Telegram bots — validated by per-customer webhook_secret
        .route(
            "/webhook/telegram/{customer_id}",
            post(handle_customer_webhook).layer(RequestBodyLimitLayer::new(64 * 1024)),
        )
        // Send: Containers POST outbound messages (validated by Bearer token)
        .route(
            "/send",
            post(handle_send)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    require_bearer_token,
                ))
                .layer(RequestBodyLimitLayer::new(256 * 1024)),
        )
        // Webhook: GitHub App sends events here (validated by HMAC-SHA256 signature)
        .route(
            "/webhook/github",
            post(github::handle_github_webhook).layer(RequestBodyLimitLayer::new(256 * 1024)),
        )
        // A2A protocol proxy (API key auth)
        .route(
            "/a2a/{customer_id}/{agent_name}",
            post(a2a_routes::handle_a2a_proxy).layer(RequestBodyLimitLayer::new(2 * 1024 * 1024)),
        )
        .route(
            "/a2a/{customer_id}/{agent_name}/agent.json",
            get(a2a_routes::handle_a2a_agent_card),
        )
        // DLQ endpoints (Bearer token auth, same as /send)
        .route(
            "/webhook/dlq",
            get(handle_dlq_list).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer_token,
            )),
        )
        .route(
            "/webhook/dlq/{delivery_id}/replay",
            post(handle_dlq_replay).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer_token,
            )),
        )
        .route(
            "/webhook/dlq/replay-all",
            post(handle_dlq_replay_all).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer_token,
            )),
        )
        // Orchestrator inbox (mika#1189) — bidirectional SSE channel for
        // spawn→orchestrator coordination. Bearer-token auth (same as /send).
        .route(
            "/orchestrator/inbox/{orchestrator_id}/message",
            post(orchestrator_inbox::handle_post_message)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    require_bearer_token,
                ))
                .layer(RequestBodyLimitLayer::new(256 * 1024)),
        )
        .route(
            "/orchestrator/inbox/{orchestrator_id}/stream",
            get(orchestrator_inbox::handle_stream).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer_token,
            )),
        )
        // Admin: customer registration (mika#1609)
        .route(
            "/admin/customers",
            post(handle_register_customer)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    require_bearer_token,
                ))
                .layer(RequestBodyLimitLayer::new(16 * 1024)),
        )
        // Health probes and version (no auth)
        .route("/health", get(handle_readiness))
        .route("/readyz", get(handle_readiness))
        .route("/livez", get(handle_liveness))
        .route("/version", get(handle_version))
        // Security headers on all responses
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // inject_request_meta must be inner to TraceLayer so that on the response
        // path it inserts RequestMeta into extensions BEFORE on_response reads them.
        .layer(middleware::from_fn(inject_request_meta))
        // Request logging — health probes at DEBUG, everything else at INFO
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &http::Request<_>| {
                    let path = request.uri().path();
                    let method = request.method();
                    if is_health_probe(path) {
                        tracing::debug_span!("http_request", %method, path)
                    } else {
                        tracing::info_span!("http_request", %method, path)
                    }
                })
                .on_response(
                    |response: &http::Response<_>, latency: Duration, span: &tracing::Span| {
                        let status = response.status().as_u16();
                        let is_debug = span
                            .metadata()
                            .is_some_and(|m| *m.level() == tracing::Level::DEBUG);
                        let (method, path) = response
                            .extensions()
                            .get::<RequestMeta>()
                            .map(|m| (m.method.as_str(), m.path.as_str()))
                            .unwrap_or(("unknown", "unknown"));
                        if status >= 500 {
                            tracing::warn!(status, method, path, ?latency, "response");
                        } else if is_debug {
                            tracing::debug!(status, method, path, ?latency, "response");
                        } else {
                            tracing::info!(status, method, path, ?latency, "response");
                        }
                    },
                )
                .on_failure(
                    // NOTE: on_failure fires on connection-level failures where no
                    // response is produced. RequestMeta is carried via response
                    // extensions, so it is unavailable here. Method and path remain
                    // accessible via the parent span's fields (nested in JSON output
                    // under `spans`). Connection-level failures are rare.
                    |error: tower_http::classify::ServerErrorsFailureClass,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::error!(
                            classification = %error,
                            ?latency,
                            "response failed"
                        );
                    },
                ),
        )
        .with_state(state)
}

/// Returns `true` for Kubernetes health/readiness/liveness probe paths.
/// These are logged at DEBUG level to reduce noise from frequent probe traffic.
fn is_health_probe(path: &str) -> bool {
    matches!(path, "/health" | "/readyz" | "/livez" | "/version")
}

// -- Webhook handler --

/// POST /webhook/telegram — receive Telegram updates.
///
/// Validates the X-Telegram-Bot-Api-Secret-Token header using constant-time comparison.
/// Returns 200 to Telegram immediately, then processes asynchronously.
/// Request body is a Telegram Update JSON object (see Telegram Bot API docs).
#[utoipa::path(
    post,
    path = "/webhook/telegram",
    responses(
        (status = 200, description = "Update accepted"),
        (status = 401, description = "Invalid webhook secret"),
        (status = 503, description = "At capacity, Telegram will retry"),
    )
)]
pub(crate) async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> StatusCode {
    // Single-bot mode only — return 404 when not configured
    let (global_telegram, global_secret) =
        match (state.telegram.as_ref(), state.webhook_secret.as_ref()) {
            (Some(tg), Some(secret)) => (tg, secret),
            _ => return StatusCode::NOT_FOUND,
        };

    // Validate secret_token header (constant-time)
    let secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq(secret, global_secret.expose_secret()) {
        return StatusCode::UNAUTHORIZED;
    }

    // Concurrency limit: shed load when at capacity (Telegram will retry)
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!("webhook at capacity, shedding load");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    let parsed = parse_update(&update);

    // Periodic cleanup of old outbound message mappings (~every 100 webhooks)
    let count = state.webhook_counter.fetch_add(1, Ordering::Relaxed);
    if count % 100 == 0 {
        let cleanup_state = state.clone();
        tokio::spawn(async move {
            cleanup_old_outbound_messages(&cleanup_state).await;
        });
    }

    // Construct a CustomerTelegramClient from the global bot token for handler dispatch.
    // In single-bot mode, all handlers use this wrapper over the global token.
    let tg = CustomerTelegramClient::new(
        state.http_client.clone(),
        global_telegram.bot_token_cloned(),
    );

    // Dispatch asynchronously — always return 200 to Telegram
    let s = state.clone();
    tokio::spawn(async move {
        let _permit = permit; // held until task completes
        dispatch_parsed_message(&s, &tg, parsed).await;
    });

    StatusCode::OK
}

// -- Per-customer webhook handler --

/// DB row for per-customer webhook lookup (includes bot_token and webhook_secret).
#[derive(Debug, sqlx::FromRow)]
struct CustomerWebhookRow {
    #[allow(dead_code)]
    id: Uuid,
    bot_token: Option<String>,
    webhook_secret: Option<String>,
}

/// POST /webhook/telegram/{customer_id} — receive per-customer Telegram updates.
///
/// Validates the `X-Telegram-Bot-Api-Secret-Token` header against the customer's
/// stored `webhook_secret`. Returns unified 401 for all failure cases (missing
/// customer, wrong secret, no bot_token) to prevent customer_id enumeration.
pub(crate) async fn handle_customer_webhook(
    State(state): State<AppState>,
    Path(customer_id): Path<Uuid>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> StatusCode {
    // Look up customer with per-customer Telegram columns
    let customer = match sqlx::query_as::<_, CustomerWebhookRow>(
        "SELECT id, bot_token, webhook_secret FROM customers WHERE id = $1",
    )
    .bind(customer_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::UNAUTHORIZED,
        Err(e) => {
            warn!(error = %e, %customer_id, "customer webhook lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    // Validate webhook_secret (constant-time). Unified 401 for missing secret,
    // invalid secret, or missing bot_token to prevent customer_id enumeration.
    let stored_secret = match customer.webhook_secret.as_deref() {
        Some(s) => s,
        None => return StatusCode::UNAUTHORIZED,
    };

    let header_secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq(header_secret, stored_secret) {
        return StatusCode::UNAUTHORIZED;
    }

    // Require bot_token to be configured
    let bot_token = match customer.bot_token {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED,
    };

    // Concurrency limit: shed load when at capacity (Telegram will retry)
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!(%customer_id, "webhook at capacity, shedding load");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    let parsed = parse_update(&update);

    // Periodic cleanup of old outbound message mappings (~every 100 webhooks)
    let count = state.webhook_counter.fetch_add(1, Ordering::Relaxed);
    if count % 100 == 0 {
        let cleanup_state = state.clone();
        tokio::spawn(async move {
            cleanup_old_outbound_messages(&cleanup_state).await;
        });
    }

    // Build per-customer telegram client
    let tg = CustomerTelegramClient::new(state.http_client.clone(), SecretString::from(bot_token));

    // Dispatch asynchronously — always return 200 to Telegram
    let s = state.clone();
    tokio::spawn(async move {
        let _permit = permit; // held until task completes
        dispatch_parsed_message(&s, &tg, parsed).await;
    });

    StatusCode::OK
}

/// Dispatch a parsed Telegram message to the appropriate handler.
/// Shared between `handle_webhook` (single-bot) and `handle_customer_webhook` (per-customer).
async fn dispatch_parsed_message(
    state: &AppState,
    tg: &CustomerTelegramClient,
    parsed: ParsedMessage,
) {
    match parsed {
        ParsedMessage::Start {
            chat_id,
            pairing_token,
        } => {
            handle_pairing(state, tg, chat_id, &pairing_token).await;
        }
        ParsedMessage::Text {
            chat_id,
            text,
            update_id,
            reply_to_message_id,
            reply_to_text,
        } => {
            handle_text_message(
                state,
                tg,
                chat_id,
                &text,
                update_id,
                reply_to_message_id,
                reply_to_text.as_deref(),
            )
            .await;
        }
        ParsedMessage::Photo {
            chat_id,
            file_id,
            caption,
            update_id,
            reply_to_message_id,
            reply_to_text,
        } => {
            handle_photo_message(
                state,
                tg,
                chat_id,
                &file_id,
                caption.as_deref(),
                update_id,
                reply_to_message_id,
                reply_to_text.as_deref(),
            )
            .await;
        }
        ParsedMessage::Document {
            chat_id,
            file_id,
            mime_type: _,
            caption,
            update_id,
            reply_to_message_id,
            reply_to_text,
        } => {
            // Image documents use the same flow as photos
            handle_photo_message(
                state,
                tg,
                chat_id,
                &file_id,
                caption.as_deref(),
                update_id,
                reply_to_message_id,
                reply_to_text.as_deref(),
            )
            .await;
        }
        ParsedMessage::BareStart { chat_id } => {
            let _ = tg
                .send_message(
                    chat_id,
                    "Welcome! If you have an invite link, please use it to get started. If you're already set up, just type a message.",
                )
                .await;
        }
        ParsedMessage::Unsupported { chat_id } => {
            // Fire-and-forget reply for non-image media (sticker/voice/video/etc.)
            let _ = tg
                .send_message(
                    chat_id,
                    "I can read text and image messages. This media type isn't supported yet.",
                )
                .await;
        }
        ParsedMessage::NoMessage => {
            // Non-message update (e.g., edited_message, channel_post) — ignore
        }
    }
}

// -- Shared routing helpers --

/// Compute container URL deterministically from customer ID.
/// When `agent_base_url` is set, routes all traffic there (local E2E testing).
/// Otherwise uses FQDN with the agents namespace for cross-namespace DNS resolution.
fn container_url(
    customer_id: &Uuid,
    agent_base_url: &Option<String>,
    agents_namespace: &str,
) -> String {
    match agent_base_url {
        Some(base) => base.clone(),
        None => format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080"),
    }
}

/// Compute container URL from a string customer ID.
/// Used by A2A proxy routes where customer_id comes from the URL path.
pub(crate) fn container_url_str(
    customer_id: &str,
    agent_base_url: Option<&str>,
    agents_namespace: &str,
) -> String {
    match agent_base_url {
        Some(base) => base.to_string(),
        None => format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080"),
    }
}

/// Look up a customer by Telegram chat ID.
/// Returns the customer row, or None after sending an appropriate reply.
async fn resolve_customer(
    state: &AppState,
    tg: &CustomerTelegramClient,
    chat_id: i64,
) -> Option<CustomerRow> {
    match sqlx::query_as::<_, CustomerRow>(
        "SELECT id, status FROM customers WHERE telegram_chat_id = $1",
    )
    .bind(chat_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(c)) => Some(c),
        Ok(None) => {
            let _ = tg
                .send_message(
                    chat_id,
                    "Please pair your account first. Use your invite link to get started.",
                )
                .await;
            None
        }
        Err(e) => {
            warn!(error = %e, chat_id, "customer lookup failed");
            reply_transient_error(tg, chat_id).await;
            None
        }
    }
}

/// Atomically claim a dedup slot for the given update_id.
/// Returns true if claimed (proceed to forward), false if already processed or on error.
async fn claim_dedup(state: &AppState, customer_id: Uuid, update_id: i64) -> bool {
    let claimed = sqlx::query(
        "UPDATE customers SET last_update_id = $1 WHERE id = $2 AND last_update_id < $1 RETURNING id",
    )
    .bind(update_id)
    .bind(customer_id)
    .fetch_optional(&state.pool)
    .await;

    match claimed {
        Ok(Some(_)) => true, // claimed -- proceed to forward
        Ok(None) => false,   // already processed by another task
        Err(e) => {
            warn!(error = %e, "dedup update failed");
            false
        }
    }
}

/// Reset the dedup slot on forwarding failure so Telegram retry can succeed.
/// Uses CAS (compare-and-swap) to prevent incorrect rollback.
async fn reset_dedup(state: &AppState, customer_id: Uuid, update_id: i64) {
    let _ = sqlx::query(
        "UPDATE customers SET last_update_id = last_update_id - 1 WHERE id = $1 AND last_update_id = $2",
    )
    .bind(customer_id)
    .bind(update_id)
    .execute(&state.pool)
    .await;
}

/// Handle the result of forwarding a message to a customer container.
/// On success: no-op. On error response: warn + reply. On network failure: reset dedup + warn + reply.
async fn handle_forward_result(
    state: &AppState,
    tg: &CustomerTelegramClient,
    result: Result<reqwest::Response, reqwest::Error>,
    chat_id: i64,
    customer_id: Uuid,
    update_id: i64,
    msg_kind: &str,
) {
    match result {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
            // Successfully forwarded
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            warn!(status, %customer_id, "container returned error for {msg_kind}");
            reply_transient_error(tg, chat_id).await;
        }
        Err(e) => {
            reset_dedup(state, customer_id, update_id).await;
            let is_connect = e.is_connect();
            warn!(error = %e, %customer_id, is_connect, "container unreachable for {msg_kind}, dedup reset");
            let msg = forward_error_message(is_connect);
            let _ = tg.send_message(chat_id, msg).await;
        }
    }
}

// -- Text message routing --

/// Route a text message to the correct customer container.
async fn handle_text_message(
    state: &AppState,
    tg: &CustomerTelegramClient,
    chat_id: i64,
    text: &str,
    update_id: i64,
    reply_to_message_id: Option<i64>,
    reply_to_text: Option<&str>,
) {
    let row = match resolve_customer(state, tg, chat_id).await {
        Some(r) => r,
        None => return,
    };

    // Suspended customer: silent drop + log
    if row.status == "suspended" {
        info!(chat_id, customer_id = %row.id, "message from suspended customer, dropping");
        return;
    }

    // Claim dedup before forwarding
    if !claim_dedup(state, row.id, update_id).await {
        return;
    }

    // Look up target agent from reply context (if replying to an agent message)
    let target_agent =
        resolve_reply_agent(state, chat_id, reply_to_message_id, reply_to_text).await;

    if reply_to_message_id.is_some() && target_agent.is_none() {
        warn!(
            chat_id,
            reply_to_message_id = ?reply_to_message_id,
            "reply routing: no agent found for replied-to message, falling back to default agent"
        );
    }

    let url = container_url(&row.id, &state.agent_base_url, &state.agents_namespace);
    let request_id = Uuid::new_v4().to_string();

    // Forward to container
    let mut payload = serde_json::json!({
        "text": text,
        "chat_id": chat_id,
        "channel": "telegram",
        "request_id": request_id
    });
    if let Some(ref agent) = target_agent {
        payload["agent"] = serde_json::Value::String(agent.clone());
    }

    let result = state
        .http_client
        .post(format!("{url}/message"))
        .bearer_auth(state.internal_token.expose_secret())
        .json(&payload)
        .timeout(Duration::from_secs(2))
        .send()
        .await;

    handle_forward_result(state, tg, result, chat_id, row.id, update_id, "text").await;
}

// -- Photo message routing --

/// Route a photo/document message to the correct customer container.
///
/// Downloads the image from Telegram, base64-encodes it, and forwards
/// alongside the caption (or synthetic text) to the agent container.
/// Dedup is claimed *after* a successful download to prevent message loss.
#[allow(clippy::too_many_arguments)]
async fn handle_photo_message(
    state: &AppState,
    tg: &CustomerTelegramClient,
    chat_id: i64,
    file_id: &str,
    caption: Option<&str>,
    update_id: i64,
    reply_to_message_id: Option<i64>,
    reply_to_text: Option<&str>,
) {
    let row = match resolve_customer(state, tg, chat_id).await {
        Some(r) => r,
        None => return,
    };

    if row.status == "suspended" {
        info!(chat_id, customer_id = %row.id, "photo from suspended customer, dropping");
        return;
    }

    // Download image BEFORE claiming dedup (prevents message loss on download failure)
    let image = match tg.download_image(file_id).await {
        Ok(img) => img,
        Err(TelegramApiError::BadRequest { ref message }) if message.contains("too large") => {
            let _ = tg
                .send_message(
                    chat_id,
                    "That image is too large for me to process. Please send a smaller photo (under 5 MB).",
                )
                .await;
            return;
        }
        Err(TelegramApiError::BadRequest { ref message }) if message.contains("unsupported") => {
            let _ = tg
                .send_message(
                    chat_id,
                    "I couldn't recognize that image format. Please send a JPEG, PNG, GIF, or WebP image.",
                )
                .await;
            return;
        }
        Err(e) => {
            error!(error = %e, chat_id, "failed to download image from Telegram");
            let _ = tg
                .send_message(
                    chat_id,
                    "Sorry, I couldn't download your photo. Please try sending it again.",
                )
                .await;
            return;
        }
    };

    let image_size = image.data.len();
    let media_type = image.media_type.clone();

    // Base64-encode the image
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&image.data);
    drop(image); // Free raw bytes

    info!(
        chat_id,
        customer_id = %row.id,
        media_type = %media_type,
        image_size,
        "downloaded and encoded image"
    );

    // Now claim dedup (download succeeded)
    if !claim_dedup(state, row.id, update_id).await {
        return;
    }

    // Look up target agent from reply context (if replying to an agent message)
    let target_agent =
        resolve_reply_agent(state, chat_id, reply_to_message_id, reply_to_text).await;

    if reply_to_message_id.is_some() && target_agent.is_none() {
        warn!(
            chat_id,
            reply_to_message_id = ?reply_to_message_id,
            "reply routing: no agent found for replied-to message, falling back to default agent"
        );
    }

    // Use caption or synthetic text for captionless photos
    let text = caption.unwrap_or("[Photo]");

    let url = container_url(&row.id, &state.agent_base_url, &state.agents_namespace);
    let request_id = Uuid::new_v4().to_string();

    // Forward to container with images array (longer timeout for large payloads)
    let mut payload = serde_json::json!({
        "text": text,
        "chat_id": chat_id,
        "channel": "telegram",
        "request_id": request_id,
        "images": [{
            "media_type": media_type,
            "data": base64_data,
        }]
    });
    if let Some(ref agent) = target_agent {
        payload["agent"] = serde_json::Value::String(agent.clone());
    }

    let result = state
        .http_client
        .post(format!("{url}/message"))
        .bearer_auth(state.internal_token.expose_secret())
        .json(&payload)
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    handle_forward_result(state, tg, result, chat_id, row.id, update_id, "photo").await;
}

// -- Admin: customer registration (mika#1609) --

#[derive(Debug, serde::Deserialize)]
struct RegisterCustomerPayload {
    customer_id: Uuid,
    name: String,
    bot_token: SecretString,
    bot_username: String,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    pairing_token_ttl_hours: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
struct RegisterCustomerResponse {
    customer_id: Uuid,
    bot_username: String,
    /// `None` for active customers whose pairing token was already consumed —
    /// the endpoint never fabricates a token it did not persist (mika#1612).
    #[serde(skip_serializing_if = "Option::is_none")]
    pairing_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pairing_url: Option<String>,
    webhook_registered: bool,
}

/// Build the response pairing fields from the DB's effective pairing token.
///
/// Returns `(None, None)` when the customer has no live pairing token (active
/// customers — the token was consumed by `handle_pairing`), so the response never
/// advertises a `pairing_url` that cannot pair (mika#1612).
fn pairing_response_fields(
    row_pairing_token: Option<&str>,
    bot_username: &str,
) -> (Option<String>, Option<String>) {
    match row_pairing_token {
        Some(token) => {
            let url = format!("https://t.me/{bot_username}?start={token}");
            (Some(token.to_string()), Some(url))
        }
        None => (None, None),
    }
}

/// Validate bot_username: alphanumeric + underscores, 1-32 chars, no leading `@`.
fn is_valid_bot_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 32
        && !username.starts_with('@')
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// POST /admin/customers — register or re-register a per-customer Telegram bot.
///
/// Creates a `customers` row with `status='provisioned'`, stores bot credentials,
/// generates pairing token and webhook secret, registers the webhook with Telegram,
/// and returns the pairing URL.
///
/// Idempotent on `customer_id`: re-registering updates bot credentials and
/// re-registers the webhook. Pairing token is only regenerated if the customer
/// is still in `provisioned` status.
async fn handle_register_customer(
    State(state): State<AppState>,
    Json(payload): Json<RegisterCustomerPayload>,
) -> impl IntoResponse {
    // Validate gateway_external_url is configured
    let gateway_url = match &state.gateway_external_url {
        Some(url) => url.clone(),
        None => {
            error!("POST /admin/customers called but gateway_external_url not configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "gateway_external_url not configured"})),
            )
                .into_response();
        }
    };

    // Validate plan
    let plan = payload.plan.as_deref().unwrap_or("standard");
    if plan != "standard" && plan != "premium" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid plan: must be 'standard' or 'premium'"})),
        )
            .into_response();
    }

    // Validate bot_username
    if !is_valid_bot_username(&payload.bot_username) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid bot_username: must be 1-32 alphanumeric/underscore characters, no leading @"})),
        )
            .into_response();
    }

    // Validate bot token via getMe and check username match
    let customer_tg = CustomerTelegramClient::new(
        state.http_client.clone(),
        SecretString::from(payload.bot_token.expose_secret().to_string()),
    );
    match customer_tg.get_me().await {
        Ok(actual_username) => {
            // Telegram usernames are case-insensitive; getMe returns canonical
            // casing, so a case-differing caller must not get a 400 (mika#1612).
            if !actual_username.eq_ignore_ascii_case(&payload.bot_username) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!(
                            "bot_username mismatch: provided '{}' but Telegram returned '{}'",
                            payload.bot_username, actual_username
                        )
                    })),
                )
                    .into_response();
            }
        }
        Err(TelegramApiError::Unauthorized) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid bot_token: Telegram returned 401 Unauthorized"})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("bot token validation failed: {e}")})),
            )
                .into_response();
        }
    }

    // Generate secrets
    let webhook_secret = generate_webhook_secret();
    let pairing_token = generate_pairing_token();
    let ttl_hours = payload.pairing_token_ttl_hours.unwrap_or(48);
    let timezone = payload.timezone.as_deref().unwrap_or("UTC");

    // Upsert customer row
    let upsert_result = sqlx::query_as::<_, UpsertCustomerRow>(
        r#"INSERT INTO customers (id, name, plan, timezone, status, bot_token, bot_username, webhook_secret, pairing_token, pairing_expires_at)
           VALUES ($1, $2, $3, $4, 'provisioned', $5, $6, $7, $8, now() + make_interval(hours => $9))
           ON CONFLICT (id) DO UPDATE SET
               name = EXCLUDED.name,
               bot_token = EXCLUDED.bot_token,
               bot_username = EXCLUDED.bot_username,
               -- Preserve the existing secret for active customers so the DB never
               -- holds a secret Telegram doesn't yet have. The new secret is only
               -- promoted after a successful setWebhook below (mika#1612).
               webhook_secret = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.webhook_secret ELSE customers.webhook_secret END,
               pairing_token = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_token ELSE customers.pairing_token END,
               pairing_expires_at = CASE WHEN customers.status = 'provisioned' THEN EXCLUDED.pairing_expires_at ELSE customers.pairing_expires_at END
           RETURNING status, pairing_token, (xmax = 0) AS was_inserted"#,
    )
    .bind(payload.customer_id)
    .bind(&payload.name)
    .bind(plan)
    .bind(timezone)
    .bind(payload.bot_token.expose_secret())
    .bind(&payload.bot_username)
    .bind(&webhook_secret)
    .bind(&pairing_token)
    // Postgres `make_interval(hours => $N)` expects int4; binding f64 (float8) is
    // only an assignment cast and fails function resolution (mika#1612). Out-of-i32
    // values fall back to the 48h default; `.max(1)` then floors zero/negative inputs
    // to 1h so a nonsensical TTL can't mint an already-expired (unpairable) token.
    .bind(i32::try_from(ttl_hours).unwrap_or(48).max(1))
    .fetch_one(&state.pool)
    .await;

    let row = match upsert_result {
        Ok(row) => row,
        Err(e) => {
            error!(error = %e, customer_id = %payload.customer_id, "failed to upsert customer");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
                .into_response();
        }
    };

    // Register webhook with Telegram (fail-open)
    let webhook_url = format!(
        "{}/webhook/telegram/{}",
        gateway_url.trim_end_matches('/'),
        payload.customer_id
    );
    let webhook_registered = match customer_tg.set_webhook(&webhook_url, &webhook_secret).await {
        Ok(()) => {
            info!(
                customer_id = %payload.customer_id,
                bot_username = %payload.bot_username,
                webhook_url = %webhook_url,
                "per-customer telegram webhook registered"
            );
            true
        }
        Err(e) => {
            warn!(
                error = %e,
                customer_id = %payload.customer_id,
                bot_username = %payload.bot_username,
                "per-customer telegram webhook registration failed (fail-open)"
            );
            false
        }
    };

    // Promote the freshly-generated secret to the DB only when Telegram has actually
    // accepted it. The upsert above preserved the old secret for active customers, so
    // a failed setWebhook leaves inbound validation working against the old secret
    // (mika#1612). Fresh inserts already hold the new secret, so skip them.
    if webhook_registered
        && !row.was_inserted
        && let Err(e) = sqlx::query("UPDATE customers SET webhook_secret = $1 WHERE id = $2")
            .bind(&webhook_secret)
            .bind(payload.customer_id)
            .execute(&state.pool)
            .await
    {
        warn!(
            error = %e,
            customer_id = %payload.customer_id,
            "failed to rotate webhook_secret after successful setWebhook; DB retains old secret"
        );
    }

    // Use the effective pairing_token from the DB. Active customers have a NULL
    // pairing_token (consumed by handle_pairing) — return no pairing fields rather
    // than fabricate a token that was never persisted (mika#1612).
    let (pairing_token_resp, pairing_url) =
        pairing_response_fields(row.pairing_token.as_deref(), &payload.bot_username);

    let status_code = if row.was_inserted {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    (
        status_code,
        Json(RegisterCustomerResponse {
            customer_id: payload.customer_id,
            bot_username: payload.bot_username,
            pairing_token: pairing_token_resp,
            pairing_url,
            webhook_registered,
        }),
    )
        .into_response()
}

#[derive(Debug, sqlx::FromRow)]
struct UpsertCustomerRow {
    #[allow(dead_code)]
    status: String,
    pairing_token: Option<String>,
    was_inserted: bool,
}

// -- Token generation --

/// Generate a cryptographic pairing token (32 random bytes, hex-encoded → 64 chars).
fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Generate a webhook secret (32 random bytes, hex-encoded → 64 chars).
fn generate_webhook_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

// -- Pairing --

/// Validate pairing token format: must be 64-char hex (32 bytes hex-encoded).
fn is_valid_pairing_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Handle /start <pairing_token> deep link for customer pairing.
async fn handle_pairing(
    state: &AppState,
    tg: &CustomerTelegramClient,
    chat_id: i64,
    pairing_token: &str,
) {
    // Reject malformed tokens before hitting the database
    if !is_valid_pairing_token(pairing_token) {
        let _ = tg
            .send_message(chat_id, "Invalid or expired invite link.")
            .await;
        return;
    }

    // Atomic: only pairs if token valid, not expired, not already paired, status is 'provisioned'
    let result = sqlx::query_as::<_, PairingResultRow>(
        r#"UPDATE customers
           SET telegram_chat_id = $1, paired_at = now(), status = 'active',
               pairing_token = NULL, pairing_expires_at = NULL
           WHERE pairing_token = $2
             AND telegram_chat_id IS NULL
             AND status = 'provisioned'
             AND pairing_expires_at > now()
           RETURNING id"#,
    )
    .bind(chat_id)
    .bind(pairing_token)
    .fetch_optional(&state.pool)
    .await;

    match result {
        Ok(Some(row)) => {
            info!(customer_id = %row.id, chat_id, "customer paired successfully");

            // Forward synthetic "Hello!" to container for onboarding
            let url = container_url(&row.id, &state.agent_base_url, &state.agents_namespace);
            let request_id = Uuid::new_v4().to_string();

            let _ = state
                .http_client
                .post(format!("{url}/message"))
                .bearer_auth(state.internal_token.expose_secret())
                .json(&serde_json::json!({
                    "text": "Hello!",
                    "chat_id": chat_id,
                    "channel": "telegram",
                    "request_id": request_id
                }))
                .timeout(Duration::from_secs(2))
                .send()
                .await;
        }
        Ok(None) => {
            // Don't reveal why — could be expired, used, or invalid
            let _ = tg
                .send_message(chat_id, "Invalid or expired invite link.")
                .await;
        }
        Err(e) => {
            if let Some(db_err) = e.as_database_error()
                && db_err.code().as_deref() == Some("23505")
            {
                let msg = if db_err
                    .constraint()
                    .is_some_and(|c| c.contains("telegram_chat_id"))
                {
                    "This Telegram account is already linked to another account."
                } else {
                    "Pairing failed. Please contact support."
                };
                let _ = tg.send_message(chat_id, msg).await;
                return;
            }
            warn!(error = %e, chat_id, "pairing query failed");
            reply_transient_error(tg, chat_id).await;
        }
    }
}

// -- Bearer auth middleware --

/// Middleware: validates `Authorization: Bearer <token>` using constant-time comparison.
async fn require_bearer_token(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(t) if constant_time_eq(t, state.internal_token.expose_secret()) => {
            next.run(req).await.into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

// -- Send handler --

/// POST /send — containers deliver outbound messages to Telegram.
///
/// Authenticated via `require_bearer_token` middleware.
#[utoipa::path(
    post,
    path = "/send",
    request_body = SendPayload,
    responses(
        (status = 200, description = "Message sent to Telegram"),
        (status = 400, description = "Invalid payload (empty or oversized text)"),
        (status = 401, description = "Missing or invalid Bearer token"),
        (status = 410, description = "Bot blocked by user"),
        (status = 429, description = "Telegram rate limit exceeded"),
        (status = 502, description = "Telegram API error"),
    ),
    security(("bearer" = []))
)]
pub(crate) async fn handle_send(
    State(state): State<AppState>,
    Json(payload): Json<SendPayload>,
) -> impl IntoResponse {
    // Validate payload
    if payload.text.is_empty() || payload.text.len() > 50_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "text must be 1-50000 bytes"})),
        )
            .into_response();
    }

    // Validate chat_id is a usable Telegram identifier (positive for private chats,
    // negative for groups/channels — but never zero, which is an invalid sentinel).
    if payload.chat_id == 0 {
        tracing::warn!(
            agent_name = ?payload.agent_name,
            request_id = ?payload.request_id,
            "chat_id=0 POST received at /send — agent should use NoChannel path"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "chat_id must be non-zero"})),
        )
            .into_response();
    }

    // Validate agent_name format (defense-in-depth at trust boundary)
    // Mirrors mika-common validate_agent_name: lowercase alphanumeric + hyphens, max 32 chars,
    // no leading/trailing hyphens, no consecutive hyphens.
    if let Some(ref name) = payload.agent_name
        && (name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || name.starts_with('-')
            || name.ends_with('-')
            || name.contains("--"))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid agent_name: must be 1-32 lowercase alphanumeric chars or hyphens, no leading/trailing/consecutive hyphens"})),
        )
            .into_response();
    }

    // Prepend agent name for identification in multi-agent setups
    let owned_text;
    let text_to_send = match &payload.agent_name {
        Some(name) => {
            owned_text = format!("[{name}] {}", payload.text);
            &owned_text
        }
        None => &payload.text,
    };

    // Resolve the Telegram client: per-customer bot token if customer_id is provided,
    // otherwise fall back to the global single-bot client.
    let tg_client: CustomerTelegramClient = if let Some(cid) = payload.customer_id {
        // Per-customer: look up bot token by primary key
        match sqlx::query_scalar::<_, Option<String>>(
            "SELECT bot_token FROM customers WHERE id = $1",
        )
        .bind(cid)
        .fetch_optional(&state.pool)
        .await
        {
            Ok(Some(Some(token))) => {
                CustomerTelegramClient::new(state.http_client.clone(), SecretString::from(token))
            }
            Ok(Some(None)) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "customer has no bot_token configured"})),
                )
                    .into_response();
            }
            Ok(None) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "customer not found"})),
                )
                    .into_response();
            }
            Err(e) => {
                error!(error = %e, %cid, "customer lookup failed for /send");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    } else {
        // Backward compat: use global single-bot client
        match state.telegram.as_ref() {
            Some(global_tg) => {
                CustomerTelegramClient::new(state.http_client.clone(), global_tg.bot_token_cloned())
            }
            None => {
                warn!(
                    agent_name = ?payload.agent_name,
                    chat_id = payload.chat_id,
                    request_id = ?payload.request_id,
                    "send failed: no customer_id provided and no global Telegram client configured"
                );
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "no customer_id provided and gateway is not in single-bot mode"})),
                )
                    .into_response();
            }
        }
    };

    // Send to Telegram (no message splitting — send as-is)
    match tg_client.send_message(payload.chat_id, text_to_send).await {
        Ok(message_id) => {
            info!(chat_id = payload.chat_id, request_id = ?payload.request_id, telegram_message_id = message_id, "sent to telegram");

            // Store outbound message mapping for reply routing
            if let Some(ref name) = payload.agent_name
                && let Err(e) = sqlx::query(
                    "INSERT INTO outbound_messages (telegram_message_id, chat_id, agent_name) VALUES ($1, $2, $3)",
                )
                .bind(message_id)
                .bind(payload.chat_id)
                .bind(name)
                .execute(&state.pool)
                .await
            {
                warn!(
                    error = %e,
                    chat_id = payload.chat_id,
                    telegram_message_id = message_id,
                    agent_name = %name,
                    "failed to store outbound message mapping — reply routing will not work for this message"
                );
            }

            StatusCode::OK.into_response()
        }
        Err(TelegramApiError::BotBlocked) => {
            warn!(chat_id = payload.chat_id, request_id = ?payload.request_id, "bot blocked by user");
            StatusCode::GONE.into_response()
        }
        Err(TelegramApiError::RateLimited { retry_after }) => {
            let mut resp_headers = HeaderMap::new();
            if let Some(secs) = retry_after {
                resp_headers.insert("retry-after", HeaderValue::from(secs));
            }
            (StatusCode::TOO_MANY_REQUESTS, resp_headers).into_response()
        }
        Err(e) => {
            warn!(chat_id = payload.chat_id, request_id = ?payload.request_id, error = %e, "telegram send failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
pub(crate) struct SendPayload {
    chat_id: i64,
    text: String,
    #[serde(default)]
    request_id: Option<String>,
    /// Customer ID for per-customer bot token lookup.
    /// When present, the gateway looks up the customer's bot token and sends
    /// via their bot. When absent, falls back to the global single-bot client.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    customer_id: Option<Uuid>,
    /// Agent name for identification in multi-agent setups.
    /// When present, outbound messages are prefixed with `[agent_name]`.
    #[serde(default)]
    agent_name: Option<String>,
}

// -- Health handlers --

/// GET /livez — Liveness probe (no auth, no DB).
///
/// Returns 200 unconditionally — the fact that HTTP is responding proves liveness.
/// Readiness (ready flag + DB) is checked by /readyz.
#[utoipa::path(
    get,
    path = "/livez",
    responses(
        (status = 200, description = "Process is alive"),
    )
)]
pub(crate) async fn handle_liveness() -> StatusCode {
    StatusCode::OK
}

/// GET /readyz, /health — Readiness probe (no auth).
///
/// Returns 200 if ready and Postgres is reachable, 503 otherwise.
#[utoipa::path(
    get,
    path = "/readyz",
    responses(
        (status = 200, description = "Ready and database reachable"),
        (status = 503, description = "Not ready or database unreachable"),
    )
)]
pub(crate) async fn handle_readiness(State(state): State<AppState>) -> StatusCode {
    if !state.ready.load(Ordering::Acquire) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

// -- DLQ endpoints --

/// Query parameters for DLQ list endpoint.
#[derive(serde::Deserialize)]
pub(crate) struct DlqListParams {
    /// Filter by status: "pending", "dead", or omit for both.
    pub status: Option<String>,
    /// Max entries to return (default 100).
    pub limit: Option<i64>,
}

/// GET /webhook/dlq — List DLQ entries (pending + dead by default).
pub(crate) async fn handle_dlq_list(
    State(state): State<AppState>,
    Query(params): Query<DlqListParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).min(1000);
    match crate::dlq::list_deliveries(&state.pool, params.status.as_deref(), limit).await {
        Ok(rows) => Json(serde_json::json!({
            "deliveries": rows,
            "count": rows.len(),
        }))
        .into_response(),
        Err(e) => {
            error!(error = %e, "DLQ list query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// POST /webhook/dlq/{delivery_id}/replay — Replay a single DLQ entry.
pub(crate) async fn handle_dlq_replay(
    State(state): State<AppState>,
    Path(delivery_id): Path<String>,
) -> impl IntoResponse {
    match crate::dlq::replay_delivery(&state, &delivery_id).await {
        Ok(Some(delivery)) => Json(serde_json::json!({
            "delivery": delivery,
        }))
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            error!(delivery_id = %delivery_id, error = %e, "DLQ replay failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Response for the replay-all endpoint.
#[derive(serde::Serialize)]
struct ReplayAllResponse {
    succeeded: u32,
    failed: u32,
    total: u32,
}

/// POST /webhook/dlq/replay-all — Replay all dead DLQ entries.
pub(crate) async fn handle_dlq_replay_all(State(state): State<AppState>) -> impl IntoResponse {
    match crate::dlq::replay_all_dead(&state).await {
        Ok((succeeded, failed)) => Json(ReplayAllResponse {
            succeeded,
            failed,
            total: succeeded + failed,
        })
        .into_response(),
        Err(e) => {
            error!(error = %e, "DLQ replay-all failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// -- Reply routing --

/// Look up the agent that sent a specific outbound message, for reply routing.
/// Returns `None` if no reply context, or if the lookup fails (best-effort).
async fn resolve_reply_agent(
    state: &AppState,
    chat_id: i64,
    reply_to_message_id: Option<i64>,
    reply_to_text: Option<&str>,
) -> Option<String> {
    // Primary: parse [agent_name] from the replied-to message text
    if let Some(text) = reply_to_text
        && let Some(agent) = parse_agent_prefix(text)
    {
        debug!(chat_id, agent = %agent, "reply routing: resolved agent from text prefix");
        return Some(agent);
    }

    // Fallback: DB lookup (outbound_messages)
    let msg_id = reply_to_message_id?;
    match sqlx::query_scalar::<_, String>(
        "SELECT agent_name FROM outbound_messages WHERE telegram_message_id = $1 AND chat_id = $2",
    )
    .bind(msg_id)
    .bind(chat_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(opt) => {
            if let Some(ref agent) = opt {
                debug!(chat_id, telegram_message_id = msg_id, agent = %agent, "reply routing: resolved agent");
            }
            opt
        }
        Err(e) => {
            warn!(error = %e, chat_id, telegram_message_id = msg_id, "reply agent lookup failed");
            None
        }
    }
}

/// Purge outbound message mappings older than 7 days.
/// Called periodically from webhook handler to avoid unbounded table growth.
async fn cleanup_old_outbound_messages(state: &AppState) {
    if let Err(e) = sqlx::query(
        "DELETE FROM outbound_messages WHERE ctid IN (SELECT ctid FROM outbound_messages WHERE created_at < now() - interval '7 days' LIMIT 1000)",
    )
    .execute(&state.pool)
    .await
    {
        debug!(error = %e, "outbound_messages cleanup failed");
    }
}

// -- Helpers --

/// Constant-time string comparison using the `subtle` crate.
fn constant_time_eq(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// User-facing message for transient errors (timeout, broken pipe, etc.).
const TRANSIENT_ERROR_MSG: &str = "I'm having trouble right now. Please try again in a moment.";

/// User-facing message when the agent container is unreachable (connection refused, DNS failure).
const OFFLINE_ERROR_MSG: &str = "Your Mika assistant is currently offline. \
     Please contact your administrator or check your subscription status \
     at console.getmika.ai.";

/// Classify a forwarding error into a user-facing reply message.
/// Connect errors (connection refused, DNS failure) indicate the agent is offline.
/// Other errors (timeout, broken pipe) are transient.
fn forward_error_message(is_connect: bool) -> &'static str {
    if is_connect {
        OFFLINE_ERROR_MSG
    } else {
        TRANSIENT_ERROR_MSG
    }
}

/// Send a generic transient error reply (fire-and-forget).
async fn reply_transient_error(tg: &CustomerTelegramClient, chat_id: i64) {
    let _ = tg.send_message(chat_id, TRANSIENT_ERROR_MSG).await;
}

// -- DB row types (for sqlx runtime queries) --

#[derive(Debug, sqlx::FromRow)]
struct CustomerRow {
    id: Uuid,
    status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct PairingResultRow {
    id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_health_probe() {
        assert!(is_health_probe("/health"));
        assert!(is_health_probe("/readyz"));
        assert!(is_health_probe("/livez"));
        assert!(is_health_probe("/version"));
        assert!(!is_health_probe("/webhook/telegram"));
        assert!(!is_health_probe("/send"));
        assert!(!is_health_probe("/a2a/customer/agent"));
        assert!(!is_health_probe("/healthy")); // substring mismatch
        assert!(!is_health_probe("/"));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "wrong"));
        assert!(!constant_time_eq("short", "longer_string"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_is_valid_pairing_token() {
        let valid = generate_pairing_token();
        assert!(is_valid_pairing_token(&valid));
        assert!(!is_valid_pairing_token("too-short"));
        assert!(!is_valid_pairing_token(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        )); // 64 chars, not hex
        assert!(!is_valid_pairing_token("")); // empty
    }

    #[test]
    fn test_container_url_default() {
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let url = container_url(&id, &None, "mika-agents");
        assert_eq!(
            url,
            "http://mika-12345678-1234-1234-1234-123456789abc.mika-agents.svc.cluster.local:8080"
        );
    }

    #[test]
    fn test_container_url_override() {
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let url = container_url(
            &id,
            &Some("http://localhost:8080".to_string()),
            "mika-agents",
        );
        assert_eq!(url, "http://localhost:8080");
    }

    #[test]
    fn test_send_payload_without_agent_name() {
        let json = r#"{"chat_id": 42, "text": "hello"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.chat_id, 42);
        assert_eq!(payload.text, "hello");
        assert!(payload.agent_name.is_none());
    }

    #[test]
    fn test_send_payload_with_agent_name() {
        let json = r#"{"chat_id": 42, "text": "hello", "agent_name": "mika-dev"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.agent_name.as_deref(), Some("mika-dev"));
    }

    #[test]
    fn test_send_payload_with_underscore_agent_name() {
        // Underscores are accepted by serde deserialization but will be rejected
        // by handle_send validation (only lowercase alphanumeric + hyphens allowed)
        let json = r#"{"chat_id": 42, "text": "hello", "agent_name": "my_agent"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.agent_name.as_deref(), Some("my_agent"));
    }

    #[test]
    fn test_agent_name_validation_rules() {
        // Helper to check if a name passes validation (mirrors handle_send logic)
        fn is_valid_agent_name(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 32
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !name.starts_with('-')
                && !name.ends_with('-')
                && !name.contains("--")
        }

        // Valid names
        assert!(is_valid_agent_name("mika"));
        assert!(is_valid_agent_name("mika-dev"));
        assert!(is_valid_agent_name("agent1"));
        assert!(is_valid_agent_name("my-agent-42"));

        // Invalid: uppercase
        assert!(!is_valid_agent_name("Mika"));
        assert!(!is_valid_agent_name("MIKA"));
        assert!(!is_valid_agent_name("mikaA"));

        // Invalid: underscore
        assert!(!is_valid_agent_name("my_agent"));

        // Invalid: leading/trailing hyphen
        assert!(!is_valid_agent_name("-mika"));
        assert!(!is_valid_agent_name("mika-"));

        // Invalid: consecutive hyphens
        assert!(!is_valid_agent_name("mika--dev"));

        // Invalid: empty
        assert!(!is_valid_agent_name(""));

        // Invalid: too long (> 32 chars)
        assert!(!is_valid_agent_name("a]234567890123456789012345678901234"));
        assert!(is_valid_agent_name("a2345678901234567890123456789012")); // exactly 32

        // Invalid: special characters
        assert!(!is_valid_agent_name("agent!"));
        assert!(!is_valid_agent_name("agent name"));
    }

    #[test]
    fn test_container_url_env_scoped_namespace() {
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let url = container_url(&id, &None, "mika-agents-prd");
        assert_eq!(
            url,
            "http://mika-12345678-1234-1234-1234-123456789abc.mika-agents-prd.svc.cluster.local:8080"
        );
    }

    #[test]
    fn test_container_url_str_uses_fqdn() {
        let url = container_url_str("abc-123", None, "mika-agents");
        assert_eq!(
            url,
            "http://mika-abc-123.mika-agents.svc.cluster.local:8080"
        );
    }

    #[test]
    fn test_container_url_str_base_url_overrides() {
        let url = container_url_str("abc-123", Some("http://localhost:9090"), "mika-agents");
        assert_eq!(url, "http://localhost:9090");
    }

    #[test]
    fn test_send_payload_chat_id_validation_rules() {
        // Mirrors the handle_send chat_id validation: `if payload.chat_id == 0`
        // returns 400. Telegram chat IDs are non-zero: positive for private chats,
        // negative for groups/channels. Zero is an invalid sentinel (#580).
        //
        // Note: handle_send requires AppState (Postgres + TelegramClient) which
        // is not available in unit tests. This test mirrors the validation logic
        // directly, same pattern as test_agent_name_validation_rules above.
        fn is_valid_chat_id(id: i64) -> bool {
            id != 0
        }

        // Valid: positive (private chat)
        assert!(is_valid_chat_id(12345));
        assert!(is_valid_chat_id(1));
        assert!(is_valid_chat_id(i64::MAX));

        // Valid: negative (group/channel)
        assert!(is_valid_chat_id(-100_123_456_789));
        assert!(is_valid_chat_id(-1));
        assert!(is_valid_chat_id(i64::MIN));

        // Invalid: zero sentinel
        assert!(!is_valid_chat_id(0));
    }

    #[test]
    fn test_send_payload_negative_chat_id_deserializes() {
        // Negative chat_ids are valid Telegram group/channel identifiers.
        let json = r#"{"chat_id": -100123456789, "text": "hello"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.chat_id, -100_123_456_789);
    }

    #[test]
    fn test_send_payload_with_customer_id() {
        let json = r#"{"chat_id": 42, "text": "hello", "customer_id": "12345678-1234-1234-1234-123456789abc"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert_eq!(
            payload.customer_id,
            Some(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap())
        );
    }

    #[test]
    fn test_send_payload_without_customer_id() {
        let json = r#"{"chat_id": 42, "text": "hello"}"#;
        let payload: SendPayload = serde_json::from_str(json).unwrap();
        assert!(payload.customer_id.is_none());
    }

    #[test]
    fn test_forward_error_message_connect() {
        let msg = forward_error_message(true);
        assert!(
            msg.contains("offline"),
            "connect errors should mention offline"
        );
        assert!(
            msg.contains("console.getmika.ai"),
            "should include console URL"
        );
    }

    #[test]
    fn test_forward_error_message_other() {
        let msg = forward_error_message(false);
        assert!(
            msg.contains("try again"),
            "non-connect errors should suggest retry"
        );
        assert!(
            !msg.contains("offline"),
            "non-connect errors should not mention offline"
        );
    }

    // -- generate_pairing_token / generate_webhook_secret tests --

    #[test]
    fn test_generate_pairing_token_length() {
        let token = generate_pairing_token();
        assert_eq!(token.len(), 64);
    }

    #[test]
    fn test_generate_pairing_token_unique() {
        let t1 = generate_pairing_token();
        let t2 = generate_pairing_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_generate_pairing_token_is_hex() {
        let token = generate_pairing_token();
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_pairing_response_fields_some_builds_url() {
        let (token, url) = pairing_response_fields(Some("abc123"), "mikabot");
        assert_eq!(token.as_deref(), Some("abc123"));
        assert_eq!(url.as_deref(), Some("https://t.me/mikabot?start=abc123"));
    }

    #[test]
    fn test_pairing_response_fields_none_omits_both() {
        // Active customers have a consumed (NULL) pairing token — the response must
        // not fabricate a token/url that cannot pair (mika#1612).
        let (token, url) = pairing_response_fields(None, "mikabot");
        assert_eq!(token, None);
        assert_eq!(url, None);
    }

    #[test]
    fn test_generate_webhook_secret_length() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64);
    }

    #[test]
    fn test_generate_webhook_secret_unique() {
        let s1 = generate_webhook_secret();
        let s2 = generate_webhook_secret();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_generate_webhook_secret_is_hex() {
        let secret = generate_webhook_secret();
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -- is_valid_bot_username tests --

    #[test]
    fn test_valid_bot_username() {
        assert!(is_valid_bot_username("MyTestBot"));
        assert!(is_valid_bot_username("test_bot_123"));
        assert!(is_valid_bot_username("a"));
        assert!(is_valid_bot_username("A_B_c_1"));
    }

    #[test]
    fn test_invalid_bot_username_empty() {
        assert!(!is_valid_bot_username(""));
    }

    #[test]
    fn test_invalid_bot_username_leading_at() {
        assert!(!is_valid_bot_username("@MyBot"));
    }

    #[test]
    fn test_invalid_bot_username_too_long() {
        let name = "a".repeat(33);
        assert!(!is_valid_bot_username(&name));
    }

    #[test]
    fn test_invalid_bot_username_special_chars() {
        assert!(!is_valid_bot_username("my-bot"));
        assert!(!is_valid_bot_username("my.bot"));
        assert!(!is_valid_bot_username("my bot"));
    }

    #[test]
    fn test_valid_bot_username_max_length() {
        let name = "a".repeat(32);
        assert!(is_valid_bot_username(&name));
    }
}
