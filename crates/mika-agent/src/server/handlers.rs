use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::Instrument;
use tracing::{error, info, warn};

use mika_common::claude::ImageSource;

use crate::agent::{self, AgentParams, check_onboarding};
use crate::compaction;
use crate::messaging::{GatewayMessageSender, MessageSender};

use super::json_extractor::JsonBody;
use super::state::{AgentState, AppState};
use super::types::{AcceptedResponse, HealthResponse, MessageRequest};

/// Media types accepted by the Claude API for image content blocks.
const ALLOWED_IMAGE_MEDIA_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// GET /health — Liveness/readiness probe (no auth required).
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Server is healthy", body = HealthResponse),
        (status = 503, description = "Server is starting up", body = HealthResponse),
    )
)]
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.ready.load(Ordering::Acquire) {
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
/// Agent responses are delivered outbound via GatewayMessageSender.
#[utoipa::path(
    post,
    path = "/message",
    request_body = MessageRequest,
    responses(
        (status = 202, description = "Message accepted for async processing", body = AcceptedResponse),
        (status = 400, description = "Invalid request (empty text without images, oversized text, or unsupported image media_type)"),
        (status = 401, description = "Missing or invalid Bearer token"),
        (status = 404, description = "Agent not found"),
        (status = 429, description = "Agent is busy processing another message"),
    ),
    security(("bearer" = []))
)]
pub async fn handle_message(
    State(state): State<AppState>,
    JsonBody(mut req): JsonBody<MessageRequest>,
) -> impl IntoResponse {
    // Validate input: text may be empty when images are present (e.g. image-only sends).
    let has_images = req.images.as_ref().is_some_and(|imgs| !imgs.is_empty());

    if req.text.is_empty() && !has_images {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "text must not be empty when no images are provided"
            })),
        )
            .into_response();
    }

    if req.text.len() > 50_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "text must be at most 50000 characters"
            })),
        )
            .into_response();
    }

    // Validate image media types against allowlist
    if let Some(images) = &req.images {
        for img in images {
            if !ALLOWED_IMAGE_MEDIA_TYPES.contains(&img.media_type.as_str()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!(
                            "unsupported media_type '{}'; allowed types: {}",
                            img.media_type,
                            ALLOWED_IMAGE_MEDIA_TYPES.join(", ")
                        )
                    })),
                )
                    .into_response();
            }
        }
    }

    // Resolve agent state (Arc clone — cheap atomic increment)
    let agent_state = match state.resolve_agent(&req.agent) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "request_id": req.request_id,
                    "error": format!("agent '{}' not found", req.agent)
                })),
            )
                .into_response();
        }
    };

    // Try to acquire the agent lock (non-blocking)
    let lock = match agent_state.agent_lock.clone().try_lock_owned() {
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
    let _ = agent_state
        .db
        .set_customer_config("chat_id", &req.chat_id.to_string())
        .await;

    let request_id = req.request_id.clone();

    // Convert gateway image payloads to Claude API ImageSource (move, not clone)
    let user_images: Vec<ImageSource> = req
        .images
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|img| ImageSource {
            source_type: "base64".to_string(),
            media_type: img.media_type,
            data: img.data,
        })
        .collect();

    // Spawn flush of previously failed sends in parallel (best-effort, non-blocking)
    let flush_state = state.clone();
    let flush_agent = agent_state.clone();
    tokio::spawn(
        async move {
            flush_failed_sends(&flush_state, &flush_agent).await;
        }
        .instrument(tracing::info_span!("flush_failed_sends")),
    );

    // Spawn async agent processing with request_id span for log correlation
    let s = state.clone();
    let a = agent_state.clone();
    let span = tracing::info_span!("process_message", request_id = %request_id);
    tokio::spawn(
        async move {
            let _lock = lock; // Hold lock for duration of agent loop

            // Hot-reload skills if the dirty flag was set by a previous turn
            let skills = if a.skills_dirty.load(Ordering::Acquire) {
                a.skills_dirty.store(false, Ordering::Release);
                let new = Arc::new(crate::skills::SkillRegistry::from_dir(
                    &a.home_dir.join("skills"),
                ));
                *a.skills.lock().unwrap() = new.clone();
                new
            } else {
                a.skills.lock().unwrap().clone()
            };

            let session_id = uuid::Uuid::new_v4().to_string();
            let is_onboarding = check_onboarding(&a.db).await;

            let sender = GatewayMessageSender::new(
                s.gateway_url.clone(),
                s.internal_token.clone(),
                a.db.clone(),
                s.http_client.clone(),
                Some(req.request_id.clone()),
            );
            let sender_arc: Arc<dyn MessageSender> = Arc::new(sender);

            let params = AgentParams {
                db: &a.db,
                claude: &s.claude,
                tools: &s.tools,
                skills: &skills,
                user_message: &req.text,
                channel_type: &req.channel,
                session_id: &session_id,
                home_dir: &a.home_dir,
                is_onboarding,
                message_sender: Some(sender_arc.clone()),
                skip_compaction: true,
                embedding_client: a.embedding_client.as_ref(),
                thinking: None,
                user_images: &user_images,
                brave_api_key: s.brave_api_key.as_deref(),
                skills_dirty: &a.skills_dirty,
                mcp_manager: a.mcp_manager.as_ref(),
                global_home_dir: Some(&s.global_home_dir),
            };

            match agent::run_agent(&params).await {
                Ok(output) => {
                    if let Some(response) = output.text {
                        info!("agent loop completed");
                        if let Err(e) = sender_arc.send(&response).await {
                            error!(error = %e, "failed to send response");
                        }
                    } else {
                        info!("agent loop completed (no text response)");
                        if let Err(e) = sender_arc.send(agent::EMPTY_RESPONSE_FALLBACK).await {
                            error!(error = %e, "failed to send fallback response");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "agent loop failed");
                    let _ = sender_arc
                        .send("Sorry, I had a hiccup processing your message. Could you try again?")
                        .await;
                }
            }

            // Spawn compaction outside the lock
            drop(_lock);
            let db = a.db.clone();
            let claude = s.claude.clone();
            tokio::spawn(
                async move {
                    if let Err(e) = compaction::maybe_compact(&db, &claude).await {
                        warn!(error = %e, "post-turn compaction failed");
                    }
                }
                .instrument(tracing::info_span!("compaction")),
            );
        }
        .instrument(span),
    );

    (
        StatusCode::ACCEPTED,
        Json(AcceptedResponse {
            request_id,
            status: "accepted".to_string(),
        }),
    )
        .into_response()
}

/// Flush previously failed outbound sends (best-effort, up to 5).
async fn flush_failed_sends(state: &AppState, agent_state: &AgentState) {
    let sends = match agent_state.db.get_pending_failed_sends(5).await {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };

    let sender = GatewayMessageSender::new(
        state.gateway_url.clone(),
        state.internal_token.clone(),
        agent_state.db.clone(),
        state.http_client.clone(),
        None,
    );

    for send in sends {
        match sender.send(&send.text).await {
            Ok(()) => {
                let _ = agent_state.db.delete_failed_send(send.id).await;
                info!(id = send.id, "flushed failed send");
            }
            Err(_) => {
                let _ = agent_state.db.increment_failed_send_retry(send.id).await;
            }
        }
    }
}
