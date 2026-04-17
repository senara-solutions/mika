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

use crate::agent;
use crate::compaction;
use crate::messaging::{GatewayMessageSender, MessageSender};
use crate::task_engine::types::{task_status, trigger_type};

use super::ci_success_handler;
use super::json_extractor::JsonBody;
use super::state::{AgentState, AppState};
use super::types::{
    AcceptedResponse, HealthResponse, MessageRequest, TaskCancelRequest, TaskCancelResponse,
    TaskCompleteRequest, TaskCompleteResponse,
};
use super::verdict_handler::{VerdictAction, try_handle_pr_review_verdict};
use super::webhook_queue::{
    self, DEFERRAL_TIMEOUT, DeferredWebhook, correlate_webhook, should_defer_webhook,
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

    // Webhook deferral check (#528): if this is a GitHub webhook targeting a work item
    // with an in-flight callback, queue it instead of processing immediately.
    if req.channel == "github"
        && let Some(correlation) = correlate_webhook(&req.text)
        && let Some((work_item_id, event_desc)) =
            should_defer_webhook(&agent_state.db, &correlation).await
    {
        let now = std::time::Instant::now();
        let deadline = now + DEFERRAL_TIMEOUT;
        let request_id = req.request_id.clone();

        // Emit audit event
        if let Err(e) = agent_state
            .db
            .log_audit_event(
                "system",
                "webhook_deferred",
                &format!("task:{work_item_id}"),
                None,
                Some(&event_desc),
                Some(&format!(
                    "Webhook deferred: in-flight callback for work item {work_item_id}"
                )),
                None,
            )
            .await
        {
            warn!(error = %e, "failed to log webhook_deferred audit event");
        }

        info!(
            request_id = %request_id,
            work_item_id = %work_item_id,
            event = %event_desc,
            "deferring webhook: work item has in-flight callback"
        );

        let deferred = DeferredWebhook {
            request: req,
            received_at: now,
            work_item_id: work_item_id.clone(),
            event_desc,
            deadline,
        };

        // Push to queue
        agent_state.webhook_queue.lock().await.push(deferred);

        // Spawn timeout task that drains only deadline-expired webhooks.
        // Uses drain_expired (not drain_for_work_item) so recently-arrived
        // webhooks that haven't hit their individual deadline are preserved.
        let queue = agent_state.webhook_queue.clone();
        let agent_state_timeout = agent_state.clone();
        let state_timeout = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DEFERRAL_TIMEOUT).await;
            let expired = {
                let mut q = queue.lock().await;
                webhook_queue::drain_expired(&mut q)
            };
            if !expired.is_empty() {
                info!(
                    count = expired.len(),
                    "replaying expired deferred webhooks after timeout"
                );
                replay_deferred_webhooks(expired, &state_timeout, &agent_state_timeout).await;
            }
        });

        return (
            StatusCode::ACCEPTED,
            Json(AcceptedResponse {
                request_id,
                status: "deferred".to_string(),
            }),
        )
            .into_response();
    }

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
            run_agent_for_message(&s, &a, req, user_images, lock).await;
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
        let webhook_queue = agent_state.webhook_queue.clone();
        let agent_state_drain = agent_state.clone();
        let state_drain = state.clone();
        let parent_work_item_id = completed_task.parent_task_id.clone();
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
                Ok(()) => {
                    // Drain deferred webhooks for the parent work item (#528).
                    // Only drain on successful dispatch — when AgentBusy fires, the
                    // callback silent-agent turn has not run, so metadata (pr_url etc.)
                    // has not been persisted yet. The 60s timeout provides the safety net.
                    if let Some(ref parent_id) = parent_work_item_id {
                        let deferred = {
                            let mut q = webhook_queue.lock().await;
                            webhook_queue::drain_for_work_item(&mut q, parent_id)
                        };
                        if !deferred.is_empty() {
                            info!(
                                count = deferred.len(),
                                work_item_id = %parent_id,
                                "replaying deferred webhooks after callback completion"
                            );
                            replay_deferred_webhooks(deferred, &state_drain, &agent_state_drain)
                                .await;
                        }
                    }
                }
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
                // Drain deferred webhooks for the parent work item (#528).
                if let Some(ref parent_id) = task.parent_task_id {
                    let webhook_queue = agent_state.webhook_queue.clone();
                    let agent_state_drain = agent_state.clone();
                    let state_drain = state.clone();
                    let parent_id = parent_id.clone();
                    let dispatcher = agent_state.dispatcher.clone();
                    let tid = task_id.clone();
                    tokio::spawn(async move {
                        let deferred = {
                            let mut q = webhook_queue.lock().await;
                            webhook_queue::drain_for_work_item(&mut q, &parent_id)
                        };
                        if !deferred.is_empty() {
                            info!(
                                count = deferred.len(),
                                work_item_id = %parent_id,
                                "replaying deferred webhooks after non-resume callback completion"
                            );
                            replay_deferred_webhooks(deferred, &state_drain, &agent_state_drain)
                                .await;
                        }
                        // Check if all siblings are done → fire parent
                        dispatcher.check_and_dispatch_parent(&tid).await;
                    });
                } else {
                    // No parent — just check siblings
                    let dispatcher = agent_state.dispatcher.clone();
                    let tid = task_id.clone();
                    tokio::spawn(async move {
                        dispatcher.check_and_dispatch_parent(&tid).await;
                    });
                }
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

/// POST /tasks/{id}/cancel — Cancel a task and kill its running process (if any).
///
/// Requires internal token auth (same as /tasks/{id}/complete).
/// Returns 200 with cancellation status.
pub async fn handle_task_cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    JsonBody(req): JsonBody<TaskCancelRequest>,
) -> impl IntoResponse {
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

    match crate::task_engine::process_kill::cancel_task_and_kill(&agent_state.db, &task_id).await {
        Ok(Some(outcome)) => {
            info!(task_id = %task_id, label = %outcome.label, "task cancelled via HTTP");
            (
                StatusCode::OK,
                Json(TaskCancelResponse {
                    task_id: task_id.to_string(),
                    status: "cancelled".to_string(),
                    process_killed: outcome.process_killed,
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("task '{}' not found or not in cancellable status", task_id),
                "task_id": task_id
            })),
        )
            .into_response(),
        Err(e) => {
            error!(error = %e, task_id, "failed to cancel task");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to cancel task", "task_id": task_id})),
            )
                .into_response()
        }
    }
}

/// Replay deferred webhooks through the normal message processing path.
///
/// Each webhook is processed sequentially, acquiring the agent lock for each turn.
/// This ensures the verdict handler and LLM see consistent work item metadata.
async fn replay_deferred_webhooks(
    webhooks: Vec<DeferredWebhook>,
    state: &AppState,
    agent_state: &Arc<AgentState>,
) {
    for deferred in webhooks {
        let deferral_ms = deferred.received_at.elapsed().as_millis();
        info!(
            request_id = %deferred.request.request_id,
            work_item_id = %deferred.work_item_id,
            event = %deferred.event_desc,
            deferral_ms = deferral_ms,
            "replaying deferred webhook"
        );

        // Log replay audit event with deferral duration
        if let Err(e) = agent_state
            .db
            .log_audit_event(
                "system",
                "webhook_replayed",
                &format!("task:{}", deferred.work_item_id),
                None,
                Some(&format!("deferral_ms={deferral_ms}")),
                Some(&format!(
                    "Replaying deferred webhook: {}",
                    deferred.event_desc
                )),
                None,
            )
            .await
        {
            warn!(error = %e, "failed to log webhook_replayed audit event");
        }

        // Acquire the agent lock (blocking wait — the callback turn should have
        // released it by now, but we wait in case another replayed webhook is
        // still processing).
        let lock = agent_state.agent_lock.clone().lock_owned().await;

        // Convert images and process through the shared agent-dispatch path
        let user_images: Vec<mika_common::llm::LlmImage> = deferred
            .request
            .images
            .as_ref()
            .map(|imgs| {
                imgs.iter()
                    .map(|img| mika_common::llm::LlmImage {
                        media_type: img.media_type.clone(),
                        data: img.data.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        run_agent_for_message(state, agent_state, deferred.request, user_images, lock).await;
    }
}

/// Shared agent-dispatch logic used by both `handle_message()` (via tokio::spawn)
/// and `replay_deferred_webhooks()` (inline). Performs skills reload, session creation,
/// verdict handler interception, agent loop execution, response delivery, and compaction.
///
/// The caller provides the agent lock guard; it is held for the duration of the agent
/// loop and released before compaction.
const AGENT_ERROR_REPLY: &str =
    "Sorry, I had a hiccup processing your message. Could you try again?";

async fn run_agent_for_message(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    mut req: MessageRequest,
    user_images: Vec<mika_common::llm::LlmImage>,
    lock: tokio::sync::OwnedMutexGuard<()>,
) {
    let _lock = lock; // Hold lock for duration of agent loop
    let a = agent_state;

    // Hot-reload skills if the dirty flag was set by a previous turn
    let skills = if a.skills_dirty.load(Ordering::Acquire) {
        a.skills_dirty.store(false, Ordering::Release);
        let mut registry = crate::skills::SkillRegistry::from_dir(&a.home_dir.join("skills"));
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
    let is_onboarding = agent::check_onboarding(&a.db).await;

    let sender = GatewayMessageSender::new(
        state.gateway_url.clone(),
        state.internal_token.clone(),
        a.db.clone(),
        state.http_client.clone(),
        Some(req.request_id.clone()),
        Some(a.db.agent_id().to_string()),
        None,
    );
    let sender_arc: Arc<dyn MessageSender> = Arc::new(sender);

    // Structural verdict handler: intercept PR review webhooks before
    // the LLM turn and act on VERDICT: pass deterministically (#524).
    // NOTE: This depends on the gateway's format_event_text() output
    // format for pull_request_review events.
    if req.channel == "github" {
        // Resolve per-agent GitHub token (PAT > App > None),
        // matching run_agent() pattern at agent.rs:1243 (#561).
        let verdict_github_token = a
            .settings
            .resolve_github_token(a.github_app.as_deref())
            .await;
        let action = try_handle_pr_review_verdict(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
            Some(&sender_arc),
            &session_id,
            &req.request_id,
        )
        .await;
        match action {
            VerdictAction::Handled { pre_digest } => {
                req.text = pre_digest;
            }
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => {
                req.text = format!("{e}{}", req.text);
            }
            VerdictAction::Passthrough { enrichment: None } => {}
        }

        // Structural CI success handler: intercept check_suite.completed(success)
        // webhooks and re-evaluate merge eligibility for PRs with pending QA pass (#571).
        // Order-independent — each handler self-selects on event type.
        let ci_action = ci_success_handler::try_handle_ci_success(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
            Some(&sender_arc),
            &session_id,
            &req.request_id,
        )
        .await;
        match ci_action {
            VerdictAction::Handled { pre_digest } => {
                req.text = pre_digest;
            }
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => {
                req.text = format!("{e}{}", req.text);
            }
            VerdictAction::Passthrough { enrichment: None } => {}
        }
    }

    let params = agent::AgentParams {
        db: &a.db,
        llm: a.llm.as_ref(),
        tools: &state.tools,
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
        brave_api_key: state.brave_api_key.as_deref(),
        github_token: a.settings.agent_github_token(),
        github_app: a.github_app.as_deref(),
        skills_dirty: &a.skills_dirty,
        mcp_manager: a.mcp_manager.as_ref(),
        global_home_dir: Some(&state.global_home_dir),
        is_callback_turn: false,
        settings: Some(&a.settings),
        trace_id: Some(req.request_id.clone()),
        correlated_task_id: None,
        internal: false,
    };

    match agent::run_agent(&params).await {
        Ok(output) => {
            if let Some(response) = output.text {
                info!("agent loop completed");
                match sender_arc.send(&response).await {
                    Ok(crate::messaging::SendOutcome::Delivered) => {}
                    Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                        warn!(reason = %reason, "response delivery failed, saved to failed_sends");
                    }
                    Err(e) => {
                        error!(error = %e, "failed to send response");
                    }
                }
            } else {
                info!("agent loop completed (no text response)");
                match sender_arc.send(agent::EMPTY_RESPONSE_FALLBACK).await {
                    Ok(crate::messaging::SendOutcome::Delivered) => {}
                    Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                        warn!(reason = %reason, "fallback response delivery failed, saved to failed_sends");
                    }
                    Err(e) => {
                        error!(error = %e, "failed to send fallback response");
                    }
                }
            }
        }
        Err(e) => {
            error!(error = %e, "agent loop failed");
            let _ = sender_arc.send(AGENT_ERROR_REPLY).await;
        }
    }

    // Spawn compaction outside the lock
    drop(_lock);
    let db = a.db.clone();
    let llm = a.llm.clone();
    tokio::spawn(
        async move {
            if let Err(e) = compaction::maybe_compact(&db, llm.as_ref()).await {
                warn!(error = %e, "post-turn compaction failed");
            }
        }
        .instrument(tracing::info_span!("compaction")),
    );
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
            Ok(crate::messaging::SendOutcome::Delivered) => {
                let _ = agent_state.db.delete_failed_send(send.id).await;
                info!(id = send.id, "flushed failed send");
            }
            Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                warn!(id = send.id, reason = %reason, "failed send flush: delivery failed again");
                let _ = agent_state.db.increment_failed_send_retry(send.id).await;
            }
            Err(e) => {
                warn!(id = send.id, error = %e, "failed send flush: sender error");
                let _ = agent_state.db.increment_failed_send_retry(send.id).await;
            }
        }
    }
}
