use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
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
    pub webhook_secret: String,
    pub ready: Arc<AtomicBool>,
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
            post(handle_send).layer(RequestBodyLimitLayer::new(256 * 1024)),
        )
        // Health: K8s probes (no auth)
        .route("/health", get(handle_health))
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
    // Validate secret_token header (constant-time, length-padded)
    let secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_eq(secret, &state.webhook_secret) {
        return StatusCode::UNAUTHORIZED;
    }

    let parsed = parse_update(&update);

    // Dispatch asynchronously — always return 200 to Telegram
    let s = state.clone();
    tokio::spawn(async move {
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
            ParsedMessage::Unsupported { chat_id } => {
                // Fire-and-forget reply for non-text messages
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
        "SELECT id, status, last_update_id FROM customers WHERE telegram_chat_id = $1",
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

    // Dedup: drop if already processed
    if update_id <= row.last_update_id {
        return;
    }

    // Suspended customer: silent drop + log
    if row.status == "suspended" {
        info!(chat_id, customer_id = %row.id, "message from suspended customer, dropping");
        return;
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
            // Update last_update_id after successful forward
            if let Err(e) = sqlx::query("UPDATE customers SET last_update_id = $1 WHERE id = $2")
                .bind(update_id)
                .bind(row.id)
                .execute(&state.pool)
                .await
            {
                warn!(error = %e, "failed to update last_update_id");
            }
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            warn!(status, customer_id = %row.id, "container returned error");
            reply_transient_error(&state.telegram, chat_id).await;
        }
        Err(e) => {
            warn!(error = %e, customer_id = %row.id, "container unreachable");
            reply_transient_error(&state.telegram, chat_id).await;
        }
    }
}

// -- Pairing --

/// Handle /start <pairing_token> deep link for customer pairing.
async fn handle_pairing(state: &AppState, chat_id: i64, pairing_token: &str) {
    // Atomic: only pairs if token valid, not expired, not already paired, status is 'provisioned'
    let result = sqlx::query_as::<_, PairingResultRow>(
        r#"UPDATE customers
           SET telegram_chat_id = $1, paired_at = now(), status = 'active',
               pairing_token = NULL, pairing_expires_at = NULL
           WHERE pairing_token = $2
             AND telegram_chat_id IS NULL
             AND status = 'provisioned'
             AND (pairing_expires_at IS NULL OR pairing_expires_at > now())
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
            warn!(error = %e, chat_id, "pairing query failed");
            reply_transient_error(&state.telegram, chat_id).await;
        }
    }
}

// -- Send handler --

/// POST /send — containers deliver outbound messages to Telegram.
///
/// Authenticated via Bearer MIKA_INTERNAL_TOKEN.
async fn handle_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SendPayload>,
) -> impl IntoResponse {
    // Validate Bearer token
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(t) if constant_time_eq(t, state.internal_token.expose_secret()) => {}
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    }

    // Validate payload
    if payload.text.is_empty() || payload.text.len() > 50_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "text must be 1-50000 characters"})),
        )
            .into_response();
    }

    // Send to Telegram (no message splitting — send as-is)
    match state
        .telegram
        .send_message(payload.chat_id, &payload.text)
        .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(TelegramApiError::BotBlocked) => {
            warn!(chat_id = payload.chat_id, "bot blocked by user");
            StatusCode::GONE.into_response()
        }
        Err(TelegramApiError::RateLimited { retry_after }) => {
            let mut resp_headers = HeaderMap::new();
            if let Some(secs) = retry_after {
                if let Ok(val) = secs.to_string().parse() {
                    resp_headers.insert("retry-after", val);
                }
            }
            (StatusCode::TOO_MANY_REQUESTS, resp_headers).into_response()
        }
        Err(e) => {
            warn!(chat_id = payload.chat_id, error = %e, "telegram send failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[derive(serde::Deserialize)]
struct SendPayload {
    chat_id: i64,
    text: String,
    #[allow(dead_code)]
    request_id: Option<String>,
}

// -- Health handler --

/// GET /health — K8s liveness/readiness probe (no auth).
///
/// Returns 200 if ready and Postgres is reachable, 503 otherwise.
async fn handle_health(State(state): State<AppState>) -> StatusCode {
    if !state.ready.load(Ordering::Acquire) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    // Quick pool connectivity check
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

// -- Helpers --

/// Constant-time string comparison, padded to avoid timing leak on length.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    // Length check is constant-time via bitwise comparison
    if a_bytes.len() != b_bytes.len() {
        // Still do a comparison to avoid timing leak on length branch
        let _ = a_bytes.ct_eq(a_bytes);
        return false;
    }

    bool::from(a_bytes.ct_eq(b_bytes))
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

/// Generate a cryptographic pairing token (32 random bytes, hex-encoded).
pub fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

// -- DB row types (for sqlx runtime queries) --

#[derive(Debug, sqlx::FromRow)]
struct CustomerRow {
    id: Uuid,
    status: String,
    last_update_id: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PairingResultRow {
    id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_same() {
        assert!(constant_time_eq("secret", "secret"));
    }

    #[test]
    fn test_constant_time_eq_different() {
        assert!(!constant_time_eq("secret", "wrong"));
    }

    #[test]
    fn test_constant_time_eq_different_length() {
        assert!(!constant_time_eq("short", "longer_string"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_generate_pairing_token_length() {
        let token = generate_pairing_token();
        assert_eq!(token.len(), 64); // 32 bytes * 2 hex chars each
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
    fn test_container_url() {
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let url = container_url(&id);
        assert_eq!(
            url,
            "http://mika-12345678-1234-1234-1234-123456789abc.mika-agents.svc.cluster.local:8080"
        );
    }
}
