use axum::{Json, extract::State, http::StatusCode, middleware::Next, response::IntoResponse};
use secrecy::ExposeSecret;
use subtle::ConstantTimeEq;

use super::state::AppState;

/// Extract the bearer token from the Authorization header.
fn extract_bearer(req: &axum::extract::Request) -> Option<&str> {
    req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Constant-time comparison of a candidate token against a secret.
fn token_matches(candidate: &str, secret: &secrecy::SecretString) -> bool {
    bool::from(candidate.as_bytes().ct_eq(secret.expose_secret().as_bytes()))
}

/// Middleware that validates the `Authorization: Bearer <token>` header
/// against the internal token only. Used for mutation endpoints (`/message`,
/// `/tasks/{id}/complete`) that must be restricted to gateway-to-agent traffic.
pub async fn require_internal_token(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    match extract_bearer(&req) {
        Some(t) if token_matches(t, &state.internal_token) => {
            next.run(req).await.into_response()
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

/// Middleware that validates the `Authorization: Bearer <token>` header
/// against either the dashboard token or the internal token. Used for
/// read-only dashboard routes (`/api/v1/*`).
///
/// If `MIKA_DASHBOARD_TOKEN` is not configured, only the internal token
/// is accepted (backwards-compatible behavior).
pub async fn require_dashboard_or_internal_token(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    let authorized = match extract_bearer(&req) {
        Some(t) => {
            // Internal token always works (superuser)
            if token_matches(t, &state.internal_token) {
                true
            } else if let Some(ref dash_token) = state.dashboard_token {
                // Dashboard token accepted for these routes
                token_matches(t, dash_token)
            } else {
                false
            }
        }
        None => false,
    };

    if authorized {
        next.run(req).await.into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}
