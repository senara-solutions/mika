use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use tokio::sync::broadcast;
use tracing::{debug, error, info};
use uuid::Uuid;

use mika_a2a::jsonrpc::{
    A2aMethod, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    METHOD_NOT_FOUND, TASK_NOT_CANCELABLE, TASK_NOT_FOUND,
};
use mika_a2a::params::{MessageSendParams, TaskIdParams, TaskQueryParams};
use mika_a2a::state_machine::TaskStateMachine;
use mika_a2a::streaming::{StreamEvent, TaskStatusUpdateEvent};
use mika_a2a::types::{Message, Part, Role, TaskState, TaskStatus};

use crate::a2a_card::build_agent_card;
use crate::a2a_db::extract_text_from_parts;
use crate::agent::{self, AgentParams, check_onboarding};
use crate::server::state::{AgentState, AppState};

/// Guard that removes a broadcaster entry from the DashMap when dropped.
/// Ensures cleanup happens even if the spawned task panics.
struct BroadcasterGuard {
    map: Arc<DashMap<String, broadcast::Sender<StreamEvent>>>,
    key: String,
}

impl Drop for BroadcasterGuard {
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}

/// Handle A2A JSON-RPC POST requests.
pub async fn handle_a2a_jsonrpc(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    debug!(method = %request.method, agent = %agent_name, "A2A JSON-RPC request");

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            request.id.clone(),
            JsonRpcError::with_message(
                mika_a2a::jsonrpc::INVALID_REQUEST,
                "Invalid JSON-RPC version",
            ),
        ))
        .into_response();
    }

    // Resolve agent
    let agent_state = match state.resolve_agent(&agent_name).await {
        Some(a) => a,
        None => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(
                    INVALID_PARAMS,
                    format!("Agent not found: {agent_name}"),
                ),
            ))
            .into_response();
        }
    };

    // Parse method
    let method = match A2aMethod::parse(&request.method) {
        Some(m) => m,
        None => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(METHOD_NOT_FOUND),
            ))
            .into_response();
        }
    };

    // Dispatch
    match method {
        A2aMethod::MessageSend => handle_message_send(&state, &agent_state, request).await,
        A2aMethod::MessageStream => handle_message_stream(&state, &agent_state, request).await,
        A2aMethod::TasksGet => handle_tasks_get(&agent_state, request).await,
        A2aMethod::TasksCancel => handle_tasks_cancel(&agent_state, request).await,
        A2aMethod::PushNotificationConfigSet => handle_push_config_set(&agent_state, request).await,
        A2aMethod::PushNotificationConfigGet => handle_push_config_get(&agent_state, request).await,
        A2aMethod::PushNotificationConfigList => {
            handle_push_config_list(&agent_state, request).await
        }
        A2aMethod::PushNotificationConfigDelete => {
            handle_push_config_delete(&agent_state, request).await
        }
        A2aMethod::TasksResubscribe => {
            handle_tasks_resubscribe(&state, &agent_state, request).await
        }
    }
}

/// Run the agent loop for an A2A request, similar to handle_message.
///
/// `stream_ctx` is the per-task broadcast context the caller (typically
/// `handle_message_stream`) uses to publish SSE frames. It bundles the
/// sender, task_id, and optional context_id so `process_tool_calls` can
/// emit `ToolCallStart` / `ToolCallResult` frames (mika#1731 wire;
/// mika#1757 emission). Non-streaming callers (`message/send`) pass `None`.
async fn run_a2a_agent(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    session_id: &str,
    input_text: &str,
    task_id: &str,
    stream_ctx: Option<Arc<mika_a2a::streaming::ToolCallStreamContext>>,
) -> Result<Option<String>, String> {
    // Hot-reload skills if dirty
    let skills = if agent_state.skills_dirty.load(Ordering::Acquire) {
        agent_state.skills_dirty.store(false, Ordering::Release);
        let mut registry =
            crate::skills::SkillRegistry::from_dir(&agent_state.home_dir.join("skills"));
        let identity = crate::prompt::load_identity(&agent_state.home_dir);
        if let Some(ref allowlist) = identity.skills.allowlist {
            registry.apply_identity_allowlist(allowlist);
        }
        if let Ok(overrides) = agent_state
            .db
            .get_skill_overrides(agent_state.db.agent_id())
            .await
        {
            registry.apply_overrides(&overrides);
        }
        registry.log_summary();
        let new = Arc::new(registry);
        *agent_state.skills.lock().unwrap() = new.clone();
        new
    } else {
        agent_state.skills.lock().unwrap().clone()
    };

    let is_onboarding = check_onboarding(&agent_state.db).await;

    let params = AgentParams {
        db: &agent_state.db,
        llm: agent_state.llm.as_ref(),
        tools: &state.tools,
        skills: &skills,
        user_message: input_text,
        channel_type: "a2a",
        session_id,
        home_dir: &agent_state.home_dir,
        is_onboarding,
        message_sender: None, // A2A responses go back via JSON-RPC, not outbound messaging
        skip_compaction: true,
        embedding_client: agent_state.embedding_client.as_ref(),
        thinking: None,
        user_images: &[],
        brave_api_key: state.brave_api_key.as_deref(),
        github_token: state.github_token.as_deref(),
        github_app: agent_state.github_app.as_deref(),
        skills_dirty: &agent_state.skills_dirty,
        mcp_manager: agent_state.mcp_manager.as_ref(),
        global_home_dir: Some(&state.global_home_dir),
        is_callback_turn: false,
        settings: Some(&agent_state.settings),
        trace_id: Some(task_id.to_string()),
        correlated_task_id: None,
        internal: false,
        pr_reviews_posted: Some(&state.pr_reviews_posted),
        stream_ctx,
    };

    match agent::run_agent(&params).await {
        Ok(output) => Ok(output.text),
        Err(e) => Err(e.to_string()),
    }
}

/// Handle `message/send` — synchronous message processing via the real agent loop.
async fn handle_message_send(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: MessageSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Acquire agent lock to prevent concurrent agent loops
    let _lock_guard = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INTERNAL_ERROR, "Agent is busy"),
            ))
            .into_response();
        }
    };

    let task_id = Uuid::new_v4().to_string();
    let context_id = params.message.context_id.clone();
    let return_immediately = params
        .configuration
        .as_ref()
        .and_then(|c| c.return_immediately)
        .unwrap_or(false);

    // Create task in DB (creates task row, session, and mapping)
    let session_id = match agent_state
        .db
        .a2a_create_task(&task_id, context_id.as_deref())
        .await
    {
        Ok(sid) => sid,
        Err(e) => {
            error!(error = %e, "failed to create A2A task");
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    if return_immediately {
        // Return task in submitted state immediately
        match agent_state.db.a2a_build_task(&task_id, None).await {
            Ok(Some(task)) => {
                let result = serde_json::to_value(&task).unwrap_or_default();
                Json(JsonRpcResponse::success(request.id, result)).into_response()
            }
            Ok(None) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response(),
            Err(e) => {
                error!(error = %e, "failed to build A2A task");
                Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response()
            }
        }
    } else {
        // Process synchronously: transition to working, run agent loop, return completed task
        let _ = agent_state
            .db
            .a2a_update_task_state(&task_id, "working")
            .await;

        let input_text = extract_text_from_parts(&params.message.parts);

        // Run the real agent loop. Non-streaming `message/send` path — no
        // broadcast subscriber, so pass `None`.
        match run_a2a_agent(state, agent_state, &session_id, &input_text, &task_id, None).await {
            Ok(_) => {
                let _ = agent_state
                    .db
                    .a2a_update_task_state(&task_id, "completed")
                    .await;

                info!(task_id = %task_id, "A2A task completed via agent loop");
            }
            Err(e) => {
                error!(error = %e, task_id = %task_id, "A2A agent loop failed");
                let _ = agent_state
                    .db
                    .a2a_update_task_state(&task_id, "failed")
                    .await;
            }
        }

        match agent_state
            .db
            .a2a_build_task(
                &task_id,
                params.configuration.as_ref().and_then(|c| c.history_length),
            )
            .await
        {
            Ok(Some(task)) => {
                let result = serde_json::to_value(&task).unwrap_or_default();
                Json(JsonRpcResponse::success(request.id, result)).into_response()
            }
            Ok(None) => Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response(),
            Err(e) => {
                error!(error = %e, "failed to build A2A task");
                Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response()
            }
        }
    }
}

/// Handle `message/stream` — SSE streaming response via the real agent loop.
async fn handle_message_stream(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: MessageSendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Acquire agent lock — moved into the spawned task to hold for duration of agent loop
    let lock_guard = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INTERNAL_ERROR, "Agent is busy"),
            ))
            .into_response();
        }
    };

    let task_id = Uuid::new_v4().to_string();
    let context_id = params.message.context_id.clone();

    // Create task in DB
    let session_id = match agent_state
        .db
        .a2a_create_task(&task_id, context_id.as_deref())
        .await
    {
        Ok(sid) => sid,
        Err(e) => {
            error!(error = %e, "failed to create A2A task");
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    // Create broadcast channel for this task
    let (tx, rx) = broadcast::channel::<StreamEvent>(32);
    let task_id_clone = task_id.clone();

    // Store broadcaster
    state.a2a_broadcasters.insert(task_id.clone(), tx.clone());

    // Spawn task processing with real agent loop
    let state_clone = state.clone();
    let agent_state_clone = Arc::clone(agent_state);
    let input_text = extract_text_from_parts(&params.message.parts);
    let broadcasters = Arc::clone(&state.a2a_broadcasters);
    tokio::spawn(async move {
        let _lock_guard = lock_guard; // Hold agent lock for duration of agent loop
        let _broadcaster_guard = BroadcasterGuard {
            map: broadcasters,
            key: task_id_clone.clone(),
        };
        // Transition to working
        let _ = agent_state_clone
            .db
            .a2a_update_task_state(&task_id_clone, "working")
            .await;

        let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task_id_clone.clone(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state: TaskState::Working,
                message: None,
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
            },
            is_final: false,
            metadata: None,
        }));

        // Run the real agent loop. Streaming path (mika#1731 wire, mika#1757
        // emission) — bundle the per-task broadcaster + task_id + context_id
        // into a ToolCallStreamContext so process_tool_calls can inject
        // ToolCallStart / ToolCallResult frames as tools dispatch. The Arc
        // wrapper enables cheap threading through run_loop.
        let stream_ctx_for_agent: Option<Arc<mika_a2a::streaming::ToolCallStreamContext>> =
            Some(Arc::new(mika_a2a::streaming::ToolCallStreamContext::new(
                Arc::new(tx.clone()),
                task_id_clone.clone(),
                context_id.clone(),
            )));
        match run_a2a_agent(
            &state_clone,
            &agent_state_clone,
            &session_id,
            &input_text,
            &task_id_clone,
            stream_ctx_for_agent,
        )
        .await
        {
            Ok(response_text) => {
                let text = response_text.unwrap_or_else(|| "Task completed.".to_string());
                let response_message = Message {
                    message_id: Uuid::new_v4().to_string(),
                    role: Role::Agent,
                    parts: vec![Part::Text {
                        text,
                        metadata: None,
                    }],
                    context_id: context_id.clone(),
                    task_id: Some(task_id_clone.clone()),
                    metadata: None,
                    reference_task_ids: None,
                    extensions: None,
                    kind: "message".to_string(),
                };

                let _ = agent_state_clone
                    .db
                    .a2a_update_task_state(&task_id_clone, "completed")
                    .await;

                // Send completion event
                let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
                    task_id: task_id_clone.clone(),
                    context_id,
                    status: TaskStatus {
                        state: TaskState::Completed,
                        message: Some(response_message),
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                    },
                    is_final: true,
                    metadata: None,
                }));
            }
            Err(e) => {
                error!(error = %e, task_id = %task_id_clone, "A2A streaming agent loop failed");
                let _ = agent_state_clone
                    .db
                    .a2a_update_task_state(&task_id_clone, "failed")
                    .await;

                let _ = tx.send(StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
                    task_id: task_id_clone.clone(),
                    context_id,
                    status: TaskStatus {
                        state: TaskState::Failed,
                        message: None,
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                    },
                    is_final: true,
                    metadata: None,
                }));
            }
        }
    });

    // Return SSE stream
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx);
    let event_stream = tokio_stream::StreamExt::map(stream, |event| {
        let ev = match event {
            Ok(stream_event) => {
                let data = serde_json::to_string(&stream_event).unwrap_or_default();
                Event::default().data(data)
            }
            Err(_) => {
                // Channel closed
                Event::default().data("")
            }
        };
        Ok::<_, Infallible>(ev)
    });

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Handle `tasks/get`.
async fn handle_tasks_get(agent_state: &Arc<AgentState>, request: JsonRpcRequest) -> Response {
    let params: TaskQueryParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state
        .db
        .a2a_build_task(&params.id, params.history_length)
        .await
    {
        Ok(Some(task)) => {
            let result = serde_json::to_value(&task).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_FOUND),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to get A2A task");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/cancel`.
async fn handle_tasks_cancel(agent_state: &Arc<AgentState>, request: JsonRpcRequest) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    let state_str = match agent_state.db.a2a_get_task_state(&params.id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response();
        }
        Err(e) => {
            error!(error = %e, "failed to get A2A task state");
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    };

    let current_state: TaskState =
        match serde_json::from_value(serde_json::Value::String(state_str)) {
            Ok(s) => s,
            Err(_) => {
                return Json(JsonRpcResponse::error(
                    request.id,
                    JsonRpcError::from_code(INTERNAL_ERROR),
                ))
                .into_response();
            }
        };

    if !TaskStateMachine::can_transition(current_state, TaskState::Canceled) {
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_CANCELABLE),
        ))
        .into_response();
    }

    if let Err(e) = agent_state
        .db
        .a2a_update_task_state(&params.id, "canceled")
        .await
    {
        error!(error = %e, "failed to cancel A2A task");
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(INTERNAL_ERROR),
        ))
        .into_response();
    }

    match agent_state.db.a2a_build_task(&params.id, None).await {
        Ok(Some(task)) => {
            let result = serde_json::to_value(&task).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(TASK_NOT_FOUND),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to build A2A task");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/set`.
async fn handle_push_config_set(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let config: mika_a2a::types::TaskPushNotificationConfig =
        match serde_json::from_value(request.params.clone()) {
            Ok(c) => c,
            Err(e) => {
                return Json(JsonRpcResponse::error(
                    request.id.clone(),
                    JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
                ))
                .into_response();
            }
        };

    if let Err(e) = agent_state.db.a2a_set_push_config(&config).await {
        error!(error = %e, "failed to set push config");
        return Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::from_code(INTERNAL_ERROR),
        ))
        .into_response();
    }

    let result = serde_json::to_value(&config).unwrap_or_default();
    Json(JsonRpcResponse::success(request.id, result)).into_response()
}

/// Handle `tasks/pushNotificationConfig/get`.
async fn handle_push_config_get(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_get_push_config(&params.id).await {
        Ok(Some(config)) => {
            let result = serde_json::to_value(&config).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(None) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::with_message(TASK_NOT_FOUND, "Push notification config not found"),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to get push config");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/list`.
async fn handle_push_config_list(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_list_push_configs(&params.id).await {
        Ok(configs) => {
            let result = serde_json::to_value(&configs).unwrap_or_default();
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Err(e) => {
            error!(error = %e, "failed to list push configs");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/pushNotificationConfig/delete`.
async fn handle_push_config_delete(
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    match agent_state.db.a2a_delete_push_config(&params.id).await {
        Ok(true) => {
            let result = serde_json::json!({"id": params.id, "deleted": true});
            Json(JsonRpcResponse::success(request.id, result)).into_response()
        }
        Ok(false) => Json(JsonRpcResponse::error(
            request.id,
            JsonRpcError::with_message(TASK_NOT_FOUND, "Push notification config not found"),
        ))
        .into_response(),
        Err(e) => {
            error!(error = %e, "failed to delete push config");
            Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response()
        }
    }
}

/// Handle `tasks/resubscribe` — reconnect to an existing task's SSE stream.
async fn handle_tasks_resubscribe(
    state: &AppState,
    agent_state: &Arc<AgentState>,
    request: JsonRpcRequest,
) -> Response {
    let params: TaskIdParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::error(
                request.id.clone(),
                JsonRpcError::with_message(INVALID_PARAMS, e.to_string()),
            ))
            .into_response();
        }
    };

    // Check task exists
    match agent_state.db.a2a_get_task_state(&params.id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(TASK_NOT_FOUND),
            ))
            .into_response();
        }
        Err(e) => {
            error!(error = %e, "failed to get A2A task state");
            return Json(JsonRpcResponse::error(
                request.id,
                JsonRpcError::from_code(INTERNAL_ERROR),
            ))
            .into_response();
        }
    }

    // Get or create broadcaster
    let rx = match state.a2a_broadcasters.get(&params.id) {
        Some(tx) => tx.subscribe(),
        None => {
            // Task exists but no active broadcaster — task might be in terminal state
            // Return current task state as a single SSE event
            match agent_state.db.a2a_build_task(&params.id, None).await {
                Ok(Some(task)) => {
                    let stream = tokio_stream::once(Ok::<_, Infallible>(Event::default().data(
                        serde_json::to_string(&StreamEvent::Task(task)).unwrap_or_default(),
                    )));
                    return Sse::new(stream).into_response();
                }
                _ => {
                    return Json(JsonRpcResponse::error(
                        request.id,
                        JsonRpcError::from_code(TASK_NOT_FOUND),
                    ))
                    .into_response();
                }
            }
        }
    };

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx);
    let event_stream = tokio_stream::StreamExt::map(stream, |event| {
        let ev = match event {
            Ok(stream_event) => {
                let data = serde_json::to_string(&stream_event).unwrap_or_default();
                Event::default().data(data)
            }
            Err(_) => Event::default().data(""),
        };
        Ok::<_, Infallible>(ev)
    });

    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Handle GET request for the Agent Card.
pub async fn handle_agent_card(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
) -> Response {
    let agent_state = match state.resolve_agent(&agent_name).await {
        Some(a) => a,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
            )
                .into_response();
        }
    };

    let skills = agent_state.skills.lock().unwrap().clone();
    let card = build_agent_card(&agent_name, "Mika AI Agent", &skills, &state.gateway_url);

    Json(card).into_response()
}
