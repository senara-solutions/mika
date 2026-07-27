use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::Instrument;
use tracing::{debug, error, info, warn};

use mika_common::llm::LlmImage;

use crate::agent;
use crate::compaction;
use crate::messaging::{GatewayMessageSender, MessageSender};
use crate::task_engine::types::{task_status, trigger_type};

use super::ci_failure_handler;
use super::ci_success_handler;
use super::json_extractor::JsonBody;
use super::milestone_context_handler;
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

/// Minimum interval between `rate_limit_trip` audit-event emissions per agent
/// (mika#1710 AC3). A naive emit-on-every-429 would write tens of thousands of
/// audit rows during a flood; throttling keeps the trip visible to the
/// orchestrator without re-creating a write flood.
const RATE_LIMIT_TRIP_AUDIT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// Decide whether to emit a throttled `rate_limit_trip` audit event, recording
/// `now` as the last-emit instant when it returns `true`.
///
/// Guard-drop discipline (mika#1723): `.get(...).map(|r| *r.value())` extracts the
/// `Copy` `Instant` and releases the shard read guard BEFORE the match, so the
/// `.insert()` in the stale/absent arm never requests a write guard on a shard this
/// thread already holds shared. DashMap shards are non-reentrant `parking_lot::RwLock`;
/// holding a `Ref` across the `.insert()` self-deadlocks.
///
/// Accepted benign race: two threads may both observe stale and both insert, costing at
/// most one duplicate audit row. DO NOT restructure into a guard-holding or `.entry()`
/// shape to close this seam (mika#1723).
fn should_emit_rate_limit_audit(
    last_emitted: &DashMap<String, std::time::Instant>,
    agent_label: &str,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> bool {
    match last_emitted.get(agent_label).map(|r| *r.value()) {
        Some(last) if now.duration_since(last) < interval => false,
        _ => {
            last_emitted.insert(agent_label.to_string(), now);
            true
        }
    }
}

/// GET /health — Combined liveness+readiness probe (no auth required).
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

/// GET /healthz — Kubernetes-convention liveness probe (no auth required).
///
/// Distinct from `/health`: this is a pure liveness endpoint — "is the process
/// alive and able to serve HTTP?" — so it returns 200 unconditionally when the
/// router is running, including during startup. This matches the semantics ops
/// tooling and K8s probes expect from `/healthz` (readiness is a separate
/// concern, tracked as a follow-up if `/readyz` is ever needed). See mika#1735.
#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (status = 200, description = "Server is alive", body = HealthResponse),
    )
)]
pub async fn handle_healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            uptime_secs: None,
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
    let agent_state = match state.resolve_agent(&req.agent).await {
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

    // Webhook deferral check (#528): if this is a GitHub webhook targeting a task
    // with an in-flight callback, queue it instead of processing immediately.
    if req.channel == "github"
        && let Some(correlation) = correlate_webhook(&req.text)
        && let Some((task_id, event_desc)) =
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
                &format!("task:{task_id}"),
                None,
                Some(&event_desc),
                Some(&format!(
                    "Webhook deferred: in-flight callback for task {task_id}"
                )),
                None,
            )
            .await
        {
            warn!(error = %e, "failed to log webhook_deferred audit event");
        }

        info!(
            request_id = %request_id,
            task_id = %task_id,
            event = %event_desc,
            "deferring webhook: task has in-flight callback"
        );

        let deferred = DeferredWebhook {
            request: req,
            received_at: now,
            task_id: task_id.clone(),
            event_desc,
            deadline,
        };

        // Push to queue
        agent_state.webhook_queue.lock().await.push(deferred);

        // Spawn timeout task that drains only deadline-expired webhooks.
        // Uses drain_expired (not drain_for_task) so recently-arrived
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
            // Emit a throttled `rate_limit_trip` audit event so the busy-lock 429 is
            // visible to the orchestrator (mika#1710 AC3). This is the server side of
            // the 429-flood fix: the gateway circuit breaker (mika#1710 R1) sheds the
            // amplification; this makes the trip observable. Throttled to one row per
            // agent per interval so the audit write does not itself become a flood.
            // An empty `req.agent` resolves to the default agent (see
            // `resolve_agent`); label the trip with the effective name so the
            // throttle key and audit target are meaningful.
            let agent_label = if req.agent.is_empty() {
                state.default_agent.clone()
            } else {
                req.agent.clone()
            };
            let now = std::time::Instant::now();
            let should_emit = should_emit_rate_limit_audit(
                &state.rate_limit_audit_last,
                &agent_label,
                now,
                RATE_LIMIT_TRIP_AUDIT_INTERVAL,
            );
            if should_emit {
                let target_key = format!("agent:{agent_label}");
                let after = format!("request_id={}", req.request_id);
                if let Err(e) = agent_state
                    .db
                    .log_audit_event(
                        "system",
                        "rate_limit_trip",
                        &target_key,
                        None,
                        Some(&after),
                        Some("agent busy — message rejected with 429"),
                        None,
                    )
                    .await
                {
                    warn!(error = %e, "failed to log rate_limit_trip audit event");
                }
            }
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

    // Store chat_id only when present and non-zero. Telegram chat IDs are
    // always non-zero (positive for private chats, negative for groups/channels).
    // Non-Telegram channels (e.g., GitHub webhooks) omit chat_id entirely;
    // storing a sentinel 0 would poison the value used for outbound Telegram
    // delivery. See #580.
    if let Some(chat_id) = req.chat_id
        && chat_id != 0
    {
        let _ = agent_state
            .db
            .set_customer_config("chat_id", &chat_id.to_string())
            .await;
    }

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

    let agent_state = match state.resolve_agent(&req.agent).await {
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
        let parent_task_id = completed_task.parent_task_id.clone();
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
                        // Agent is busy — keep status as 'completed' (already set at
                        // update_task_completed above) so dispatch_undelivered_callbacks
                        // can find it on the next scan. Only set next_fire_at for the
                        // retry delay. mika#1070: resetting to 'pending' stranded
                        // callbacks — neither get_schedulable_tasks (excludes callbacks)
                        // nor get_undelivered_callback_tasks (requires completed/failed)
                        // could find them.
                        debug!(task_id = %completed_task.id, "agent busy, deferring resume_agent to callback scan in ~60s");
                        let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
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
                    // Drain deferred webhooks for the parent task (#528).
                    // Only drain on successful dispatch — when AgentBusy fires, the
                    // callback silent-agent turn has not run, so metadata (pr_url etc.)
                    // has not been persisted yet. The 60s timeout provides the safety net.
                    if let Some(ref parent_id) = parent_task_id {
                        let deferred = {
                            let mut q = webhook_queue.lock().await;
                            webhook_queue::drain_for_task(&mut q, parent_id)
                        };
                        if !deferred.is_empty() {
                            info!(
                                count = deferred.len(),
                                task_id = %parent_id,
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
                // Drain deferred webhooks for the parent task (#528).
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
                            webhook_queue::drain_for_task(&mut q, &parent_id)
                        };
                        if !deferred.is_empty() {
                            info!(
                                count = deferred.len(),
                                task_id = %parent_id,
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
    let agent_state = match state.resolve_agent(&req.agent).await {
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
/// This ensures the verdict handler and LLM see consistent task metadata.
async fn replay_deferred_webhooks(
    webhooks: Vec<DeferredWebhook>,
    state: &AppState,
    agent_state: &Arc<AgentState>,
) {
    for deferred in webhooks {
        let deferral_ms = deferred.received_at.elapsed().as_millis();
        info!(
            request_id = %deferred.request.request_id,
            task_id = %deferred.task_id,
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
                &format!("task:{}", deferred.task_id),
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
        let identity = crate::prompt::load_identity(&a.home_dir);
        if let Some(ref allowlist) = identity.skills.allowlist {
            registry.apply_identity_allowlist(allowlist);
        }
        if let Ok(overrides) = a.db.get_skill_overrides(a.db.agent_id()).await {
            registry.apply_overrides(&overrides);
        }
        registry.log_summary();
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
        a.settings.customer_id.clone(),
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
            &skills,
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
            // Engine-side dispatch fired (mika#1572 ready-label, mika#1630 verdict).
            VerdictAction::Dispatched { pre_digest, .. } => {
                req.text = pre_digest;
            }
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
            // Only the ready-label handler returns Dispatched (mika#1572).
            VerdictAction::Dispatched { .. } => {}
        }

        // Structural CI failure handler: intercept check_suite.completed(failure|timed_out)
        // webhooks, gather failure context, and prepare dispatch pre-digest (#594).
        // Order-independent — self-selects on failure/timed_out conclusions.
        let ci_failure_action = ci_failure_handler::try_handle_ci_failure(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
            Some(&sender_arc),
            &session_id,
            &req.request_id,
        )
        .await;
        match ci_failure_action {
            VerdictAction::Handled { pre_digest } => {
                req.text = pre_digest;
            }
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => {
                req.text = format!("{e}{}", req.text);
            }
            VerdictAction::Passthrough { enrichment: None } => {}
            // Only the ready-label handler returns Dispatched (mika#1572).
            VerdictAction::Dispatched { .. } => {}
        }

        // Structural ready-label dispatch handler (mika#1384): intercepts
        // `[GitHub] Issue labeled ready on …` webhooks and pre-resolves every
        // decision the LLM has historically failed to make (the
        // `run_claude_pilot_groom`/`run_claude_pilot` no-call class). Returns
        // a prescriptive pre-digest with the resolved task_id / skill / args.
        // Composes with the existing `webhook_ready_label_dispatch` INTENT_GUARDS
        // entry: this handler runs before the LLM turn; the guard runs after.
        let ready_label_action = super::ready_label_handler::try_handle_ready_label_dispatch(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
            Some(&sender_arc),
            &session_id,
            &req.request_id,
            &skills,
        )
        .await;
        match ready_label_action {
            // Engine-side dispatch already fired (mika#1572): replace the message
            // with the post-dispatch pre-digest. It starts with
            // `<ready_label_handler>`, so the `webhook_ready_label_dispatch`
            // INTENT_GUARD does not fire (the LLM has no dispatch left to make).
            VerdictAction::Dispatched { pre_digest, .. } => {
                req.text = pre_digest;
            }
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

        // Milestone-context marker injector (mika#1218): for `pull_request.closed`
        // webhooks whose correlated task has a milestone/project parent, prepend
        // a `[milestone-parent: <id>]` marker so the inline webhook_milestone_advance
        // guard in agent.rs can fire. Never returns Handled (LLM still owns the
        // advance/halt decision).
        // Phase tracking + cascade (mika#1153): also computes phase progress and
        // triggers ready-label cascade on phase rollover.
        let milestone_action = milestone_context_handler::try_handle_pr_closed_milestone_context(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
        )
        .await;
        match milestone_action {
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => {
                req.text = format!("{e}{}", req.text);
            }
            VerdictAction::Passthrough { enrichment: None } => {}
            VerdictAction::Handled { .. } => {
                unreachable!("milestone_context handler never handles");
            }
            VerdictAction::Dispatched { .. } => {
                unreachable!("milestone_context handler never dispatches");
            }
        }

        // Draft-PR ready-label cleanup handler (mika#1849): for
        // `pull_request.opened` webhooks where the PR is a draft closing one or
        // more issues, remove the leftover `ready` label from each closing
        // issue. Side-effect-only injector — never returns Handled/Dispatched
        // (same shape as milestone_context above). Self-selects on the
        // `[GitHub] PR opened:` prefix; order-independent with the handlers above.
        let draft_pr_action = super::draft_pr_opened_handler::try_handle_draft_pr_opened(
            &req.text,
            &a.db,
            verdict_github_token.as_deref(),
            &session_id,
            &req.request_id,
        )
        .await;
        match draft_pr_action {
            VerdictAction::Passthrough {
                enrichment: Some(e),
            } => {
                req.text = format!("{e}{}", req.text);
            }
            VerdictAction::Passthrough { enrichment: None } => {}
            VerdictAction::Handled { .. } => {
                unreachable!("draft_pr_opened handler never handles");
            }
            VerdictAction::Dispatched { .. } => {
                unreachable!("draft_pr_opened handler never dispatches");
            }
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
        pr_reviews_posted: Some(&state.pr_reviews_posted),
        stream_tx: None,
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
                    Ok(crate::messaging::SendOutcome::NoChannel) => {
                        warn!("response delivery skipped — no reply channel (chat_id=0)");
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
                    Ok(crate::messaging::SendOutcome::NoChannel) => {
                        warn!("fallback response delivery skipped — no reply channel (chat_id=0)");
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

/// Rows older than this are dropped without being sent (mika#1751).
const FAILED_SEND_STALE_THRESHOLD: chrono::Duration = chrono::Duration::minutes(5);

/// Prefix applied to delivered rows that were parked in `failed_sends` (mika#1751).
const FAILED_SEND_STALE_PREFIX: &str = "⏳ from earlier — ";

/// Prefix applied when `created_at` cannot be parsed. Fail-open: keep the message
/// visible with a loud marker rather than drop it silently (mika#1751,
/// mika-arch first-pass §Uncertainty 3).
const FAILED_SEND_UNPARSEABLE_PREFIX: &str = "⚠️ UNPARSEABLE TIMESTAMP — ";

/// Classification decision for a row read from `failed_sends`.
#[derive(Debug, PartialEq, Eq)]
enum FlushAction {
    /// Delete the row without sending. Reason: age exceeded the staleness
    /// threshold; the fresh turn supersedes the parked reply.
    Drop { age_secs: i64 },
    /// Send the row after prepending `prefix` to its text.
    Deliver { prefix: &'static str },
}

/// Decide what to do with a `failed_sends` row given its `created_at` timestamp.
///
/// Pure function — no I/O, no side effects. All decision policy lives here so
/// it can be exercised by unit tests without touching the gateway sender.
fn classify_failed_send(created_at: &str, now: chrono::DateTime<chrono::Utc>) -> FlushAction {
    match crate::timestamp::parse(created_at) {
        Ok(dt) => {
            let age = now - dt;
            if age > FAILED_SEND_STALE_THRESHOLD {
                FlushAction::Drop {
                    age_secs: age.num_seconds(),
                }
            } else {
                FlushAction::Deliver {
                    prefix: FAILED_SEND_STALE_PREFIX,
                }
            }
        }
        Err(_) => FlushAction::Deliver {
            prefix: FAILED_SEND_UNPARSEABLE_PREFIX,
        },
    }
}

/// Flush previously failed outbound sends (best-effort, up to 5).
///
/// Policy (mika#1751):
/// - Rows older than `FAILED_SEND_STALE_THRESHOLD` are dropped without a send.
/// - Rows within the threshold are prefixed with `FAILED_SEND_STALE_PREFIX`.
/// - Rows with an unparseable `created_at` are delivered with
///   `FAILED_SEND_UNPARSEABLE_PREFIX` (fail-open).
///
/// The `Drop` branch does NOT call `increment_failed_send_retry` — a dropped
/// row was never retried, so incrementing would be incorrect accounting.
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
        agent_state.settings.customer_id.clone(),
    );

    let now = chrono::Utc::now();
    for send in sends {
        match classify_failed_send(&send.created_at, now) {
            FlushAction::Drop { age_secs } => {
                let text_preview: String = send.text.chars().take(80).collect();
                warn!(
                    id = send.id,
                    age_secs,
                    retry_count = send.retry_count,
                    created_at = %send.created_at,
                    text_preview = %text_preview,
                    "dropping stale failed_send"
                );
                let _ = agent_state.db.delete_failed_send(send.id).await;
            }
            FlushAction::Deliver { prefix } => {
                if prefix == FAILED_SEND_UNPARSEABLE_PREFIX {
                    error!(
                        id = send.id,
                        created_at = %send.created_at,
                        "failed_sends row has unparseable created_at; delivering fail-open with warning prefix"
                    );
                } else {
                    debug!(id = send.id, "flushing failed_send with stale prefix");
                }
                let text_to_send = format!("{}{}", prefix, send.text);
                match sender.send(&text_to_send).await {
                    Ok(crate::messaging::SendOutcome::Delivered) => {
                        let _ = agent_state.db.delete_failed_send(send.id).await;
                        info!(id = send.id, "flushed failed send");
                    }
                    Ok(crate::messaging::SendOutcome::Failed { reason }) => {
                        warn!(id = send.id, reason = %reason, "failed send flush: delivery failed again");
                        let _ = agent_state.db.increment_failed_send_retry(send.id).await;
                    }
                    Ok(crate::messaging::SendOutcome::NoChannel) => {
                        // Permanent condition — delete the entry instead of retrying.
                        // These entries were created before the NoChannel check existed.
                        let _ = agent_state.db.delete_failed_send(send.id).await;
                        warn!(
                            id = send.id,
                            "failed send flush: no reply channel (chat_id=0), deleting entry"
                        );
                    }
                    Err(e) => {
                        warn!(id = send.id, error = %e, "failed send flush: sender error");
                        let _ = agent_state.db.increment_failed_send_retry(send.id).await;
                    }
                }
            }
        }
    }
}

// ===== Skill Lifecycle Endpoints (mika#1582) =====

/// POST /api/v1/skills/{name}/promote — promote a staged skill to active.
pub async fn handle_skill_promote(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    skill_lifecycle_transition(state, name, "active").await
}

/// POST /api/v1/skills/{name}/archive — archive an active skill.
pub async fn handle_skill_archive(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    skill_lifecycle_transition(state, name, "archived").await
}

async fn skill_lifecycle_transition(
    state: AppState,
    skill_name: String,
    target_state: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    // Use default agent for lifecycle transitions.
    let Some(agent_state) = state.resolve_agent("").await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no agent found" })),
        );
    };

    // Check current lifecycle state
    let current = match agent_state
        .db
        .get_skill_lifecycle_state(&agent_state.db.agent_id, &skill_name)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("database error: {e}") })),
            );
        }
    };

    match current {
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!(
                        "skill '{skill_name}' has no lifecycle_state — \
                         bundled/marketplace skills cannot be promoted or archived"
                    )
                })),
            );
        }
        Some(ref s) if s == target_state => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "no_change",
                    "skill": skill_name,
                    "lifecycle_state": target_state,
                    "message": format!("skill '{skill_name}' is already in '{target_state}' state")
                })),
            );
        }
        _ => {}
    }

    match agent_state
        .db
        .set_skill_lifecycle_state(&agent_state.db.agent_id, &skill_name, target_state)
        .await
    {
        Ok(()) => {
            info!(
                skill = %skill_name,
                from = current.as_deref().unwrap_or("unknown"),
                to = target_state,
                "skill lifecycle state changed"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "updated",
                    "skill": skill_name,
                    "lifecycle_state": target_state,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to update lifecycle state: {e}") })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Primary regression test (mika#1723). Seed the map with a stale timestamp for
    /// one label, then fan out N threads (above typical `num_cpus`) that all call
    /// `should_emit_rate_limit_audit` for that same label concurrently. Under the
    /// pre-fix guard-holding `match` shape, the stale/absent arm's `.insert()`
    /// requested a write guard on a shard this thread already held shared and the
    /// call hung forever (futex_wait); every subsequent same-shard call queued
    /// behind it. Under the fix the read guard is dropped before the match, so all
    /// threads complete. The assertion is on *completion within a bounded timeout*,
    /// not on how many threads returned `true` — the accepted get→insert race
    /// (mika#1723 § "Accepted benign race") makes the true-count non-deterministic
    /// under extreme timing, so asserting exactly-one-true would be flaky.
    #[test]
    fn concurrent_stale_revisit_does_not_deadlock() {
        const N: usize = 16;
        let map: Arc<DashMap<String, Instant>> = Arc::new(DashMap::new());
        let label = "agent-under-contention";
        let now = Instant::now();
        // Seed a stale entry so every thread takes the `Some`-but-stale path — the
        // exact path that self-deadlocked pre-fix.
        map.insert(label.to_string(), now - Duration::from_secs(60));

        let interval = RATE_LIMIT_TRIP_AUDIT_INTERVAL;
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let map = Arc::clone(&map);
            handles.push(std::thread::spawn(move || {
                should_emit_rate_limit_audit(&map, label, Instant::now(), interval)
            }));
        }

        // Watchdog: a coordinator thread joins all workers and signals completion.
        // The main thread waits with a bounded timeout — a timeout means a worker
        // is wedged (the pre-fix deadlock), which fails the test instead of hanging
        // the whole suite.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for h in handles {
                // A panicking/wedged worker propagates here; the send below never
                // fires and the main thread's recv_timeout catches it.
                h.join().expect("worker thread panicked");
            }
            let _ = tx.send(());
        });

        rx.recv_timeout(Duration::from_secs(5))
            .expect("threads did not complete within 5s — self-deadlock regressed (mika#1723)");
    }

    /// Throttle semantics preserved: first call for a label emits, an immediate
    /// second call is suppressed (within interval), and a call past the interval
    /// emits again.
    #[test]
    fn throttle_semantics_preserved() {
        let map: DashMap<String, Instant> = DashMap::new();
        let label = "agent-a";
        let interval = Duration::from_secs(10);
        let t0 = Instant::now();

        // Absent key → insert, emit.
        assert!(should_emit_rate_limit_audit(&map, label, t0, interval));
        // Within interval → suppressed.
        assert!(!should_emit_rate_limit_audit(
            &map,
            label,
            t0 + Duration::from_secs(1),
            interval
        ));
        // Past interval → stale, re-emit.
        assert!(should_emit_rate_limit_audit(
            &map,
            label,
            t0 + interval + Duration::from_secs(1),
            interval
        ));
    }

    /// Stale-entry revisit (trigger-condition unit): a single call on a pre-seeded
    /// stale entry returns `true` and updates the stored instant to `now`. This is
    /// the exact path that self-deadlocked pre-fix, exercised single-threaded.
    #[test]
    fn stale_entry_revisit_emits_and_updates() {
        let map: DashMap<String, Instant> = DashMap::new();
        let label = "agent-b";
        let interval = Duration::from_secs(10);
        let now = Instant::now();
        let stale = now - interval - Duration::from_secs(1);
        map.insert(label.to_string(), stale);

        assert!(should_emit_rate_limit_audit(&map, label, now, interval));
        // The stored instant advanced to `now` (stale entry was overwritten).
        assert_eq!(*map.get(label).unwrap().value(), now);
    }

    // ============================================================================
    // classify_failed_send tests (mika#1751)
    // ============================================================================

    /// A row whose `created_at` predates the staleness threshold classifies as `Drop`.
    /// This is the primary fix for the "twice-hello" incident — the 3.5h-old parked
    /// reply is dropped before the sender is invoked.
    #[test]
    fn classify_failed_send_drops_row_past_threshold() {
        let now = chrono::Utc::now();
        // 10 minutes ago — well past the 5-minute threshold.
        let created_at = crate::timestamp::format(&(now - chrono::Duration::minutes(10)));
        let action = classify_failed_send(&created_at, now);
        match action {
            FlushAction::Drop { age_secs } => {
                // Age should be about 600 seconds (10 minutes). Allow a small window
                // for the `now` value having advanced between format and classify.
                assert!(
                    (599..=601).contains(&age_secs),
                    "unexpected age_secs: {age_secs}"
                );
            }
            other => panic!("expected Drop, got {other:?}"),
        }
    }

    /// A row within the staleness threshold classifies as `Deliver` with the stale
    /// prefix — the reader gets an in-order marker so the delivery reads as "from
    /// earlier" instead of a memory glitch.
    #[test]
    fn classify_failed_send_delivers_row_within_threshold_with_stale_prefix() {
        let now = chrono::Utc::now();
        // 30 seconds ago — well within the 5-minute threshold.
        let created_at = crate::timestamp::format(&(now - chrono::Duration::seconds(30)));
        let action = classify_failed_send(&created_at, now);
        assert_eq!(
            action,
            FlushAction::Deliver {
                prefix: FAILED_SEND_STALE_PREFIX
            }
        );
    }

    /// The threshold boundary: a row just under the threshold classifies as
    /// `Deliver` (the `>` comparison in `classify_failed_send` is strict). The
    /// `DB_FORMAT` used for `created_at` has second precision, so an exact
    /// `now - threshold` timestamp round-trips through parse to a value
    /// slightly BEFORE `now - threshold` (sub-second truncation) and tips into
    /// `Drop`. This test uses `threshold - 5 seconds` to sit well within the
    /// bound. The paired "past threshold" case is covered by
    /// `classify_failed_send_drops_row_past_threshold`.
    #[test]
    fn classify_failed_send_just_under_threshold_delivers() {
        let now = chrono::Utc::now();
        let created_at = crate::timestamp::format(
            &(now - FAILED_SEND_STALE_THRESHOLD + chrono::Duration::seconds(5)),
        );
        let action = classify_failed_send(&created_at, now);
        assert_eq!(
            action,
            FlushAction::Deliver {
                prefix: FAILED_SEND_STALE_PREFIX
            }
        );
    }

    /// Fail-open policy for unparseable timestamps: the row is delivered with a
    /// screaming `UNPARSEABLE TIMESTAMP` prefix rather than dropped. This flips
    /// the pass-1 default (Drop) per mika-arch first-pass §Uncertainty 3 —
    /// dropping is silent data loss unless the accompanying `error!` is
    /// actively paged, which it isn't at this frequency.
    #[test]
    fn classify_failed_send_unparseable_timestamp_delivers_fail_open() {
        let now = chrono::Utc::now();
        let action = classify_failed_send("not-a-timestamp", now);
        assert_eq!(
            action,
            FlushAction::Deliver {
                prefix: FAILED_SEND_UNPARSEABLE_PREFIX
            }
        );
    }

    /// The two prefixes are distinct — a delivery consumer that greps for one
    /// won't accidentally match the other.
    #[test]
    fn failed_send_prefixes_are_distinct() {
        assert_ne!(FAILED_SEND_STALE_PREFIX, FAILED_SEND_UNPARSEABLE_PREFIX);
        assert!(FAILED_SEND_STALE_PREFIX.contains("from earlier"));
        assert!(FAILED_SEND_UNPARSEABLE_PREFIX.contains("UNPARSEABLE"));
    }
}
