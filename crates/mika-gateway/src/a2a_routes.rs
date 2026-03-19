use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use secrecy::ExposeSecret;
use tracing::{debug, error};

use mika_a2a::jsonrpc::{A2aMethod, JsonRpcRequest};

use crate::a2a_auth::validate_a2a_api_key;
use crate::routes::AppState;

/// Extract API key from request headers.
fn extract_api_key(headers: &axum::http::HeaderMap) -> Option<String> {
    // Try x-api-key header first
    if let Some(key) = headers.get("x-api-key") {
        return key.to_str().ok().map(|s| s.to_string());
    }
    // Fall back to Authorization: Bearer
    if let Some(auth) = headers.get(header::AUTHORIZATION)
        && let Ok(auth_str) = auth.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
    {
        return Some(token.to_string());
    }
    None
}

/// Handle A2A JSON-RPC proxy requests.
pub async fn handle_a2a_proxy(
    State(state): State<AppState>,
    Path((customer_id, agent_name)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    // Validate API key
    let api_key = match extract_api_key(&headers) {
        Some(k) => k,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing API key"})),
            )
                .into_response();
        }
    };

    let key_customer = match validate_a2a_api_key(&state.pool, &api_key).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid API key"})),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "API key validation error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Internal error"})),
            )
                .into_response();
        }
    };

    // Ensure API key matches the requested customer
    if key_customer != customer_id {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "API key does not match customer"})),
        )
            .into_response();
    }

    // Forward to agent container
    let container = crate::routes::container_url_str(&customer_id, state.agent_base_url.as_deref());
    let forward_url = format!("{container}/a2a/{agent_name}");
    debug!(forward_url = %forward_url, method = %request.method, "proxying A2A request");

    // Check if this is a streaming method
    let is_streaming = matches!(
        A2aMethod::parse(&request.method),
        Some(A2aMethod::MessageStream | A2aMethod::TasksResubscribe)
    );

    let body = match serde_json::to_string(&request) {
        Ok(b) => b,
        Err(e) => {
            error!(error = %e, "failed to serialize request");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Serialization error"})),
            )
                .into_response();
        }
    };

    let mut req = state
        .http_client
        .post(&forward_url)
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!("Bearer {}", state.internal_token.expose_secret()),
        )
        .body(body);

    if is_streaming {
        req = req.header("accept", "text/event-stream");
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "failed to forward A2A request");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Agent unavailable"})),
            )
                .into_response();
        }
    };

    let status = resp.status();

    if is_streaming && status.is_success() {
        // Stream SSE response through
        let byte_stream = resp.bytes_stream();
        let body = Body::from_stream(byte_stream);
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("connection", "keep-alive")
            .body(body)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    } else {
        // Forward JSON response
        let response_body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                error!(error = %e, "failed to read agent response");
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": "Failed to read response"})),
                )
                    .into_response();
            }
        };

        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Body::from(response_body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

/// Handle GET request for Agent Card (proxied from agent container).
pub async fn handle_a2a_agent_card(
    State(state): State<AppState>,
    Path((customer_id, agent_name)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Validate API key
    let api_key = match extract_api_key(&headers) {
        Some(k) => k,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing API key"})),
            )
                .into_response();
        }
    };

    match validate_a2a_api_key(&state.pool, &api_key).await {
        Ok(Some(c)) if c == customer_id => {}
        Ok(Some(_)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "API key does not match customer"})),
            )
                .into_response();
        }
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid API key"})),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "API key validation error");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Internal error"})),
            )
                .into_response();
        }
    }

    // Forward to agent container
    let container = crate::routes::container_url_str(&customer_id, state.agent_base_url.as_deref());
    let forward_url = format!("{container}/a2a/{agent_name}/agent.json");

    let resp = match state
        .http_client
        .get(&forward_url)
        .header(
            "authorization",
            format!("Bearer {}", state.internal_token.expose_secret()),
        )
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "failed to fetch agent card");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Agent unavailable"})),
            )
                .into_response();
        }
    };

    let status = resp.status();
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            error!(error = %e, "failed to read agent card response");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": "Failed to read response"})),
            )
                .into_response();
        }
    };

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
