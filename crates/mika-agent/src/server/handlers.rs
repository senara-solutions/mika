use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::atomic::Ordering;
use tracing::{error, info};

use crate::agent::{self, AgentParams, check_onboarding};
use crate::compaction;

use super::state::AppState;
use super::types::{AcceptedResponse, HealthResponse, MessageRequest};

/// GET /health — K8s liveness/readiness probe (no auth required).
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.ready.load(Ordering::Relaxed) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "starting".to_string(),
                uptime_secs: None,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            uptime_secs: Some(state.startup_time.elapsed().as_secs()),
        }),
    )
}

/// POST /message — gateway forwards a user message for async processing.
///
/// Returns 202 Accepted immediately, then spawns the agent loop in background.
/// Agent responses are delivered via GatewayMessageSender (wired in Phase 2 PR 3).
pub async fn handle_message(
    State(state): State<AppState>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.text.is_empty() || req.text.len() > 50_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "text must be 1-50000 characters"
            })),
        )
            .into_response();
    }

    // Try to acquire the agent lock (non-blocking)
    let lock = match state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "request_id": req.request_id,
                    "error": "agent busy"
                })),
            )
                .into_response();
        }
    };

    // Store chat_id on every message (for outbound sends)
    let _ = state
        .db
        .set_customer_config("chat_id", &req.chat_id.to_string())
        .await;

    let request_id = req.request_id.clone();

    // Spawn async agent processing
    let s = state.clone();
    tokio::spawn(async move {
        let _lock = lock; // Hold lock for duration of agent loop

        let session_id = uuid::Uuid::new_v4().to_string();
        let is_onboarding = check_onboarding(&s.db).await;

        // Build message_sender — GatewayMessageSender will be wired in PR 3.
        // For now, agent responses are saved to DB only.
        let message_sender = None;

        let params = AgentParams {
            db: &s.db,
            claude: &s.claude,
            tools: &s.tools,
            user_message: &req.text,
            channel_type: &req.channel,
            session_id: &session_id,
            home_dir: &s.home_dir,
            is_onboarding,
            message_sender,
        };

        match agent::run_agent(&params).await {
            Ok(_response) => {
                info!(request_id = req.request_id, "agent loop completed");
                // TODO (PR 3): send _response via GatewayMessageSender
            }
            Err(e) => {
                error!(error = %e, request_id = req.request_id, "agent loop failed");
                // TODO (PR 3): send error message via GatewayMessageSender
            }
        }

        // Spawn compaction outside the lock
        drop(_lock);
        let db = s.db.clone();
        let claude = s.claude.clone();
        tokio::spawn(async move {
            if let Err(e) = compaction::maybe_compact(&db, &claude).await {
                tracing::warn!(error = %e, "post-turn compaction failed");
            }
        });
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!(AcceptedResponse {
            request_id,
            status: "accepted".to_string(),
        })),
    )
        .into_response()
}
