use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
};
use secrecy::{ExposeSecret, SecretString};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{info, warn};
use uuid::Uuid;

use crate::telegram::{
    ParsedMessage, TelegramApiError, TelegramClient, TelegramUpdate, parse_update,
};

// -- AppState --

/// Shared application state for the gateway.
/// All fields are Clone-able (owned or Arc-wrapped).
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub telegram: TelegramClient,
    pub http_client: reqwest::Client,
    pub internal_token: SecretString,
    pub webhook_secret: SecretString,
    pub ready: Arc<AtomicBool>,
    pub webhook_semaphore: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("internal_token", &"[REDACTED]")
            .field("webhook_secret", &"[REDACTED]")
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
        // K8s probes (no auth)
        .route("/health", get(handle_readiness))
        .route("/readyz", get(handle_readiness))
        .route("/livez", get(handle_liveness))
        // Security headers on all responses
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .with_state(state)
}

// -- Webhook handler --

/// POST /webhook/telegram — receive Telegram updates.
///
/// Validates the X-Telegram-Bot-Api-Secret-Token header using constant-time comparison.
/// Returns 200 to Telegram immediately, then processes asynchronously.
async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> StatusCode {
    // Validate secret_token header (constant-time)
    let secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq(secret, state.webhook_secret.expose_secret()) {
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

    // Dispatch asynchronously — always return 200 to Telegram
    let s = state.clone();
    tokio::spawn(async move {
        let _permit = permit; // held until task completes
        match parsed {
            ParsedMessage::Start {
                chat_id,
                pairing_token,
            } => {
                handle_pairing(&s, chat_id, &pairing_token).await;
            }
            ParsedMessage::Text {
                chat_id,
                text,
                update_id,
            } => {
                handle_text_message(&s, chat_id, &text, update_id).await;
            }
            ParsedMessage::BareStart { chat_id } => {
                let _ = s
                    .telegram
                    .send_message(
                        chat_id,
                        "Welcome! If you have an invite link, please use it to get started. If you're already set up, just type a message.",
                    )
                    .await;
            }
            ParsedMessage::Unsupported { chat_id } => {
                // Fire-and-forget reply for non-text media (photo/sticker/voice)
                let _ = s
                    .telegram
                    .send_message(
                        chat_id,
                        "I can only read text messages right now. Please type your message.",
                    )
                    .await;
            }
            ParsedMessage::NoMessage => {
                // Non-message update (e.g., edited_message, channel_post) — ignore
            }
        }
    });

    StatusCode::OK
}

// -- Text message routing --

/// Compute container URL deterministically from customer ID.
/// Eliminates SSRF — no user-controlled URLs.
fn container_url(customer_id: &Uuid) -> String {
    format!("http://mika-{customer_id}.mika-agents.svc.cluster.local:8080")
}

/// Route a text message to the correct customer container.
async fn handle_text_message(state: &AppState, chat_id: i64, text: &str, update_id: i64) {
    // Look up customer by telegram_chat_id (runtime query — no DATABASE_URL needed at build time)
    let row = match sqlx::query_as::<_, CustomerRow>(
        "SELECT id, status FROM customers WHERE telegram_chat_id = $1",
    )
    .bind(chat_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            let _ = state
                .telegram
                .send_message(
                    chat_id,
                    "Please pair your account first. Use your invite link to get started.",
                )
                .await;
            return;
        }
        Err(e) => {
            warn!(error = %e, chat_id, "customer lookup failed");
            reply_transient_error(&state.telegram, chat_id).await;
            return;
        }
    };

    // Suspended customer: silent drop + log
    if row.status == "suspended" {
        info!(chat_id, customer_id = %row.id, "message from suspended customer, dropping");
        return;
    }

    // Atomic dedup: claim this update_id before forwarding.
    // If another task already claimed it, skip silently.
    let claimed = sqlx::query(
        "UPDATE customers SET last_update_id = $1 WHERE id = $2 AND last_update_id < $1 RETURNING id",
    )
    .bind(update_id)
    .bind(row.id)
    .fetch_optional(&state.pool)
    .await;

    match claimed {
        Ok(Some(_)) => {}   // claimed — proceed to forward
        Ok(None) => return, // already processed by another task
        Err(e) => {
            warn!(error = %e, "dedup update failed");
            return;
        }
    }

    let url = container_url(&row.id);
    let request_id = Uuid::new_v4().to_string();

    // Forward to container
    let result = state
        .http_client
        .post(format!("{url}/message"))
        .bearer_auth(state.internal_token.expose_secret())
        .json(&serde_json::json!({
            "text": text,
            "chat_id": chat_id,
            "channel": "telegram",
            "request_id": request_id
        }))
        .timeout(Duration::from_secs(2))
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
            // Successfully forwarded
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            warn!(status, customer_id = %row.id, "container returned error");
            reply_transient_error(&state.telegram, chat_id).await;
        }
        Err(e) => {
            // Reset dedup so Telegram retry can succeed (CAS prevents incorrect rollback)
            let _ = sqlx::query(
                "UPDATE customers SET last_update_id = last_update_id - 1 WHERE id = $1 AND last_update_id = $2",
            )
            .bind(row.id)
            .bind(update_id)
            .execute(&state.pool)
            .await;
            warn!(error = %e, customer_id = %row.id, "container unreachable, dedup reset");
            reply_transient_error(&state.telegram, chat_id).await;
        }
    }
}

// -- Pairing --

/// Validate pairing token format: must be 64-char hex (32 bytes hex-encoded).
fn is_valid_pairing_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Handle /start <pairing_token> deep link for customer pairing.
async fn handle_pairing(state: &AppState, chat_id: i64, pairing_token: &str) {
    // Reject malformed tokens before hitting the database
    if !is_valid_pairing_token(pairing_token) {
        let _ = state
            .telegram
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
            let url = container_url(&row.id);
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
            let _ = state
                .telegram
                .send_message(chat_id, "Invalid or expired invite link.")
                .await;
        }
        Err(e) => {
            if let Some(db_err) = e.as_database_error() {
                if db_err.code().as_deref() == Some("23505") {
                    let msg = if db_err
                        .constraint()
                        .is_some_and(|c| c.contains("telegram_chat_id"))
                    {
                        "This Telegram account is already linked to another account."
                    } else {
                        "Pairing failed. Please contact support."
                    };
                    let _ = state.telegram.send_message(chat_id, msg).await;
                    return;
                }
            }
            warn!(error = %e, chat_id, "pairing query failed");
            reply_transient_error(&state.telegram, chat_id).await;
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
async fn handle_send(
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

    // Send to Telegram (no message splitting — send as-is)
    match state
        .telegram
        .send_message(payload.chat_id, &payload.text)
        .await
    {
        Ok(()) => {
            info!(chat_id = payload.chat_id, request_id = ?payload.request_id, "sent to telegram");
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

#[derive(serde::Deserialize)]
struct SendPayload {
    chat_id: i64,
    text: String,
    #[serde(default)]
    request_id: Option<String>,
}

// -- Health handlers --

/// GET /livez — K8s liveness probe (no auth, no DB).
///
/// Returns 200 unconditionally — the fact that HTTP is responding proves liveness.
/// Readiness (ready flag + DB) is checked by /readyz.
async fn handle_liveness() -> StatusCode {
    StatusCode::OK
}

/// GET /readyz, /health — K8s readiness probe (no auth).
///
/// Returns 200 if ready and Postgres is reachable, 503 otherwise.
async fn handle_readiness(State(state): State<AppState>) -> StatusCode {
    if !state.ready.load(Ordering::Acquire) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

// -- Helpers --

/// Constant-time string comparison using the `subtle` crate.
fn constant_time_eq(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// Send a generic transient error reply (fire-and-forget).
async fn reply_transient_error(telegram: &TelegramClient, chat_id: i64) {
    let _ = telegram
        .send_message(
            chat_id,
            "I'm having trouble right now. Please try again in a moment.",
        )
        .await;
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

    /// Generate a cryptographic pairing token (32 random bytes, hex-encoded).
    fn generate_pairing_token() -> String {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        hex::encode(bytes)
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "wrong"));
        assert!(!constant_time_eq("short", "longer_string"));
        assert!(constant_time_eq("", ""));
    }

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
    fn test_container_url() {
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let url = container_url(&id);
        assert_eq!(
            url,
            "http://mika-12345678-1234-1234-1234-123456789abc.mika-agents.svc.cluster.local:8080"
        );
    }
}
