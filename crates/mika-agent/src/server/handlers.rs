use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::Instrument;
use tracing::{debug, error, info, warn};

use mika_common::llm::LlmImage;

use crate::agent::{self, AgentParams, check_onboarding};
use crate::compaction;
use crate::messaging::{GatewayMessageSender, MessageSender};
use crate::task_engine::types::{task_status, trigger_type};

use super::json_extractor::JsonBody;
use super::state::{AgentState, AppState};
use super::types::{
    AcceptedResponse, HealthResponse, MessageRequest, TaskCompleteRequest, TaskCompleteResponse,
};

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

    // Convert gateway image payloads to provider-agnostic LlmImage
    let user_images: Vec<LlmImage> = req
        .images
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|img| LlmImage {
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
    let span =
        tracing::info_span!(target: "mika::otel", "process_message", request_id = %request_id);
    tokio::spawn(
        async move {
            let _lock = lock; // Hold lock for duration of agent loop

            // Hot-reload skills if the dirty flag was set by a previous turn
            let skills = if a.skills_dirty.load(Ordering::Acquire) {
                a.skills_dirty.store(false, Ordering::Release);
                let mut registry =
                    crate::skills::SkillRegistry::from_dir(&a.home_dir.join("skills"));
                if let Ok(overrides) = a.db.get_skill_overrides(a.db.agent_id()).await {
                    registry.apply_overrides(&overrides);
                }
                let new = Arc::new(registry);
                *a.skills.lock().unwrap() = new.clone();
                new
            } else {
                a.skills.lock().unwrap().clone()
            };

            let session_id = uuid::Uuid::new_v4().to_string();
            if let Err(e) =
                a.db.create_session(&session_id, a.db.agent_id(), &req.channel)
                    .await
            {
                warn!(error = %e, "failed to create session");
            }
            let is_onboarding = check_onboarding(&a.db).await;

            let sender = GatewayMessageSender::new(
                s.gateway_url.clone(),
                s.internal_token.clone(),
                a.db.clone(),
                s.http_client.clone(),
                Some(req.request_id.clone()),
                Some(a.db.agent_id().to_string()),
                None,
            );
            let sender_arc: Arc<dyn MessageSender> = Arc::new(sender);

            let params = AgentParams {
                db: &a.db,
                llm: s.llm.as_ref(),
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
                is_callback_turn: false,
                settings: Some(&s.settings),
                trace_id: Some(req.request_id.clone()),
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
            let llm = s.llm.clone();
            tokio::spawn(
                async move {
                    if let Err(e) = compaction::maybe_compact(&db, llm.as_ref()).await {
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

/// POST /tasks/{id}/complete — mark a callback task complete and trigger resume_agent.
///
/// Called by background processes (exec handlers, scripts) to deliver results back
/// to the agent. Validates the task exists and has trigger_type="callback", stores
/// the result, marks the task completed, then spawns the resume_agent dispatch.
#[utoipa::path(
    post,
    path = "/tasks/{id}/complete",
    params(("id" = String, Path, description = "Task UUID")),
    request_body = TaskCompleteRequest,
    responses(
        (status = 200, description = "Task marked complete, resume_agent dispatched", body = TaskCompleteResponse),
        (status = 400, description = "Task is not a callback task or result is empty"),
        (status = 401, description = "Missing or invalid Bearer token"),
        (status = 404, description = "Task not found"),
        (status = 409, description = "Task already completed or cancelled"),
    ),
    security(("bearer" = []))
)]
pub async fn handle_task_complete(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    JsonBody(req): JsonBody<TaskCompleteRequest>,
) -> impl IntoResponse {
    if req.result.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "result must not be empty", "task_id": task_id})),
        )
            .into_response();
    }

    if req.result.len() > 100_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "result must be at most 100000 characters", "task_id": task_id})),
        )
            .into_response();
    }

    let agent_state = match state.resolve_agent(&req.agent) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", req.agent), "task_id": task_id})),
            )
                .into_response();
        }
    };

    // Load the task
    let task = match agent_state.db.get_task(&task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("task '{}' not found", task_id), "task_id": task_id})),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, task_id, "failed to load task");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to load task", "task_id": task_id})),
            )
                .into_response();
        }
    };

    // Validate trigger_type
    if task.trigger_type != trigger_type::CALLBACK {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("task '{}' has trigger_type '{}', not 'callback'", task_id, task.trigger_type),
                "task_id": task_id
            })),
        )
            .into_response();
    }

    // Validate status — only pending or in_progress tasks can be completed
    if !matches!(
        task.status.as_str(),
        task_status::PENDING | task_status::IN_PROGRESS
    ) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("task '{}' has status '{}' and cannot be completed", task_id, task.status),
                "task_id": task_id
            })),
        )
            .into_response();
    }

    // Transition to in_progress before spawning dispatch so startup_recovery can detect stuck tasks
    if let Err(e) = agent_state
        .db
        .update_task_status(&task_id, task_status::IN_PROGRESS)
        .await
    {
        error!(error = %e, task_id, "failed to transition task to in_progress");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "failed to complete task", "task_id": task_id})),
        )
            .into_response();
    }

    // If action_type is resume_agent, spawn the dispatch in background;
    // otherwise mark completed immediately.
    if task.action_type == crate::task_engine::types::action_type::RESUME_AGENT {
        // Build the task struct with result set for dispatch
        let mut completed_task = task;
        completed_task.result = Some(req.result.clone());
        completed_task.status = task_status::IN_PROGRESS.to_string();

        let db = agent_state.db.clone();
        let dispatcher = agent_state.dispatcher.clone();
        let task_id_clone = task_id.clone();
        let result_clone = req.result.clone();
        tokio::spawn(async move {
            // Persist result and mark completed in DB before dispatch
            match db
                .update_task_completed(&task_id_clone, Some(&result_clone))
                .await
            {
                Ok(false) => {
                    warn!(task_id = %task_id_clone, "task already in terminal state, skipping dispatch");
                    return;
                }
                Err(e) => {
                    warn!(task_id = %task_id_clone, error = %e, "failed to mark task completed before dispatch");
                    if let Err(db_err) = db
                        .update_task_status(&task_id_clone, task_status::FAILED)
                        .await
                    {
                        warn!(task_id = %task_id_clone, error = %db_err, "failed to mark task as failed in DB after completion error");
                    }
                    return;
                }
                Ok(true) => {}
            }

            match dispatcher.dispatch_resume_agent(&completed_task).await {
                Err(crate::task_engine::DispatchError::AgentBusy(_)) => {
                    // Check if the task has expired before re-queuing
                    let now = crate::timestamp::now();
                    let is_expired = completed_task
                        .timeout_at
                        .as_ref()
                        .is_some_and(|ts| ts.as_str() <= now.as_str());

                    if is_expired {
                        warn!(task_id = %completed_task.id, "task timed out while waiting for agent, marking failed");
                        if let Err(db_err) = db
                            .update_task_failed(
                                &completed_task.id,
                                "task timed out while waiting for agent",
                            )
                            .await
                        {
                            warn!(task_id = %completed_task.id, error = %db_err, "failed to mark timed-out task as failed in DB");
                        }
                    } else {
                        // Agent is busy — reset task to pending for the tick loop to retry
                        debug!(task_id = %completed_task.id, "agent busy, deferring resume_agent to tick loop in 30s");
                        let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
                        if let Err(e) = db
                            .update_task_status(&completed_task.id, task_status::PENDING)
                            .await
                        {
                            warn!(task_id = %completed_task.id, error = %e, "failed to reset task status to pending for retry");
                        }
                        if let Err(e) = db
                            .update_task_next_fire_at(&completed_task.id, &retry_at)
                            .await
                        {
                            warn!(task_id = %completed_task.id, error = %e, "failed to update next_fire_at for retry");
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %completed_task.id, error = %e, "resume_agent dispatch failed");
                }
                Ok(()) => {}
            }

            // Check if all siblings are done → fire parent
            dispatcher.check_and_dispatch_parent(&task_id_clone).await;
        });
    } else {
        // Non-resume_agent callback: persist result and mark completed directly
        match agent_state
            .db
            .update_task_completed(&task_id, Some(&req.result))
            .await
        {
            Ok(false) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "task already completed or not in completable state",
                        "task_id": task_id
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                error!(error = %e, task_id, "failed to mark task completed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::json!({"error": "failed to complete task", "task_id": task_id}),
                    ),
                )
                    .into_response();
            }
            Ok(true) => {
                // Check if all siblings are done → fire parent
                let dispatcher = agent_state.dispatcher.clone();
                let tid = task_id.clone();
                tokio::spawn(async move {
                    dispatcher.check_and_dispatch_parent(&tid).await;
                });
            }
        }
    }

    (
        StatusCode::OK,
        Json(TaskCompleteResponse {
            task_id: task_id.to_string(),
            status: task_status::COMPLETED.to_string(),
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
        Some(agent_state.db.agent_id().to_string()),
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
