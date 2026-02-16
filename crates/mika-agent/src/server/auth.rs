use axum::{Json, extract::State, http::StatusCode, middleware::Next, response::IntoResponse};
use secrecy::ExposeSecret;
use subtle::ConstantTimeEq;

use super::state::AppState;

/// Middleware that validates the `Authorization: Bearer <token>` header
/// using constant-time comparison. Returns 401 with JSON body if missing or wrong.
pub async fn require_internal_token(
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
        Some(t)
            if bool::from(
                t.as_bytes()
                    .ct_eq(state.internal_token.expose_secret().as_bytes()),
            ) =>
        {
            next.run(req).await.into_response()
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}
