use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::rewind;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RewindRequest {
    pub session_id: String,
    pub after_message_id: i64,
    /// Target agent name (defaults to the server's default agent if absent).
    #[serde(default)]
    pub agent: String,
    /// For cross-session rewinds, the session that initiated the rewind.
    #[serde(default)]
    pub originating_session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RewindPreviewResponse {
    pub messages: Vec<MessagePreviewResponse>,
    pub reversals: Vec<ReversalPreviewResponse>,
    pub tasks: Vec<TaskPreviewResponse>,
    pub warnings: Vec<String>,
    pub blocked: bool,
    pub block_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MessagePreviewResponse {
    pub id: i64,
    pub role: String,
    pub content_snippet: String,
}

#[derive(Debug, Serialize)]
pub struct ReversalPreviewResponse {
    pub audit_event_id: i64,
    pub tool_name: String,
    pub target_key: String,
    pub action: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct TaskPreviewResponse {
    pub task_id: String,
    pub label: String,
    pub actioned: bool,
}

#[derive(Debug, Serialize)]
pub struct RewindResultResponse {
    pub messages_deleted: usize,
    pub reversals_applied: usize,
    pub reversals_skipped: usize,
    pub tasks_deleted: usize,
    pub tasks_cancelled: usize,
    pub warnings: Vec<String>,
    pub reversal_descriptions: Vec<String>,
    pub rewind_trace_id: String,
}

fn validate_rewind_request(req: &RewindRequest) -> Result<(), (StatusCode, String)> {
    if req.session_id.is_empty() || req.session_id.len() > 1000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid session_id: must be non-empty and under 1000 chars.".to_string(),
        ));
    }
    if req.after_message_id < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid after_message_id: must be non-negative.".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct RewindResolveRequest {
    pub session_id: String,
    #[serde(default = "default_count")]
    pub count: usize,
    /// Target agent name (defaults to the server's default agent if absent).
    #[serde(default)]
    pub agent: String,
}

fn default_count() -> usize {
    1
}

#[derive(Debug, Serialize)]
pub struct RewindResolveResponse {
    pub session_id: String,
    pub after_message_id: i64,
    pub trace_ids: Vec<String>,
}

pub async fn handle_rewind_resolve(
    State(state): State<AppState>,
    Json(req): Json<RewindResolveRequest>,
) -> impl IntoResponse {
    if req.session_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id must not be empty"})),
        )
            .into_response();
    }
    if req.count == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "count must be at least 1"})),
        )
            .into_response();
    }

    let agent_state = match state.resolve_agent(&req.agent) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", req.agent)})),
            )
                .into_response();
        }
    };

    // Acquire agent lock (non-blocking)
    let _lock = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is busy. Try again when idle."})),
            )
                .into_response();
        }
    };

    match rewind::find_recent_exchanges(&agent_state.db, &req.session_id, req.count).await {
        Ok(Some(m)) => {
            let resp = RewindResolveResponse {
                session_id: m.session_id,
                after_message_id: m.anchor_message_id,
                trace_ids: m.trace_ids,
            };
            (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No exchanges found in session"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Resolve failed: {e}")})),
        )
            .into_response(),
    }
}

pub async fn handle_rewind_preview(
    State(state): State<AppState>,
    Json(req): Json<RewindRequest>,
) -> impl IntoResponse {
    let agent_state = match state.resolve_agent(&req.agent) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", req.agent)})),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) = validate_rewind_request(&req) {
        return (status, Json(serde_json::json!({"error": msg}))).into_response();
    }

    // Acquire agent lock (non-blocking)
    let _lock = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is busy. Try again when idle."})),
            )
                .into_response();
        }
    };

    match rewind::preview_rewind(&agent_state.db, &req.session_id, req.after_message_id).await {
        Ok(preview) => {
            let resp = RewindPreviewResponse {
                messages: preview
                    .messages
                    .iter()
                    .map(|m| MessagePreviewResponse {
                        id: m.id,
                        role: m.role.clone(),
                        content_snippet: m.content_snippet.clone(),
                    })
                    .collect(),
                reversals: preview
                    .reversals
                    .iter()
                    .map(|r| ReversalPreviewResponse {
                        audit_event_id: r.audit_event_id,
                        tool_name: r.tool_name.clone(),
                        target_key: r.target_key.clone(),
                        action: format!("{:?}", r.action),
                        description: r.description.clone(),
                    })
                    .collect(),
                tasks: preview
                    .tasks
                    .iter()
                    .map(|t| TaskPreviewResponse {
                        task_id: t.task_id.clone(),
                        label: t.label.clone(),
                        actioned: t.actioned,
                    })
                    .collect(),
                warnings: preview.warnings,
                blocked: preview.blocked,
                block_reason: preview.block_reason,
            };

            if resp.blocked {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::to_value(resp).unwrap()),
                )
                    .into_response()
            } else {
                (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Preview failed: {e}")})),
        )
            .into_response(),
    }
}

pub async fn handle_rewind_execute(
    State(state): State<AppState>,
    Json(req): Json<RewindRequest>,
) -> impl IntoResponse {
    let agent_state = match state.resolve_agent(&req.agent) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", req.agent)})),
            )
                .into_response();
        }
    };

    if let Err((status, msg)) = validate_rewind_request(&req) {
        return (status, Json(serde_json::json!({"error": msg}))).into_response();
    }

    // Acquire agent lock (non-blocking)
    let _lock = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is busy. Try again when idle."})),
            )
                .into_response();
        }
    };

    match rewind::execute_rewind(
        &agent_state.db,
        &req.session_id,
        req.after_message_id,
        req.originating_session_id.as_deref(),
    )
    .await
    {
        Ok(result) => {
            let resp = RewindResultResponse {
                messages_deleted: result.messages_deleted,
                reversals_applied: result.reversals_applied,
                reversals_skipped: result.reversals_skipped,
                tasks_deleted: result.tasks_deleted,
                tasks_cancelled: result.tasks_cancelled,
                warnings: result.warnings,
                reversal_descriptions: result.reversal_descriptions,
                rewind_trace_id: result.rewind_trace_id,
            };
            (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Rewind blocked") {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": msg})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("Rewind failed: {e}")})),
                )
                    .into_response()
            }
        }
    }
}
