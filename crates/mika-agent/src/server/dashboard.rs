//! Dashboard API handlers — read-only endpoints for the observability dashboard.
//!
//! DB structs (`TimelineRow`, `AgentWithStats`, `CoreMemoryEntry`, `SessionWithStats`,
//! `AuditEvent`, `Session`) derive `Serialize` + `ToSchema` directly, so no wrapper
//! response types are needed for 1:1 mappings. Only `MessageResponse` is kept because
//! it applies a `strip_base64_images` transformation.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

use crate::db::{
    self, CoreMemoryEntry, SessionMessage, Task, TaskFilters, TeamRunFilters, TeamRunIdFilter,
    TimelineFilters,
};

use super::state::AppState;

// ===== Shared Types =====

const DEFAULT_PER_PAGE: u32 = 50;
const MAX_PER_PAGE: u32 = 200;

#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

pub fn resolve_pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, u32) {
    let page = page.unwrap_or(1).clamp(1, 100_000);
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let offset = (page - 1).saturating_mul(per_page);
    (page, per_page, offset)
}

pub fn internal_error(e: anyhow::Error) -> impl IntoResponse {
    error!(error = %e, "dashboard query failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal server error"})),
    )
}

// ===== Timeline =====

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub agent_id: Option<String>,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/timeline — paginated unified timeline with filters.
pub async fn handle_timeline(
    State(state): State<AppState>,
    Query(q): Query<TimelineQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);
    let filters = TimelineFilters {
        agent_id: q.agent_id.clone(),
        event_type: q.event_type.clone(),
        trace_id: q.trace_id.clone(),
        session_id: q.session_id.clone(),
        from: q.from,
        to: q.to,
    };

    let (data, total) = match state
        .dashboard_db
        .query_timeline_with_count(filters, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/timeline/trace/:trace_id — all events for a trace.
pub async fn handle_trace_detail(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.query_timeline_by_trace(&trace_id).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/timeline/trace/:trace_id/messages — full messages for a trace.
pub async fn handle_trace_messages(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_messages_by_trace_id(&trace_id).await {
        Ok(messages) => {
            let data: Vec<MessageResponse> =
                messages.into_iter().map(MessageResponse::from).collect();
            Json(data).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Agents =====

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentDetailResponse {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub last_seen: Option<String>,
    pub created_at: String,
    pub message_count: i64,
    pub core_memory: Vec<CoreMemoryEntry>,
    pub core_memory_edits_this_session: i64,
    pub soul_md: String,
}

/// GET /api/v1/agents — list all agents with stats.
pub async fn handle_agents_list(State(state): State<AppState>) -> impl IntoResponse {
    match state.dashboard_db.list_agents_with_stats().await {
        Ok(agents) => Json(agents).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/agents/:id — agent detail with core memory and soul.md.
pub async fn handle_agent_detail(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    // Find the agent in our agents map to get home_dir
    let agent_state = match state.resolve_agent(&agent_id) {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", agent_id)})),
            )
                .into_response();
        }
    };

    // Get agent row from DB (single-row query, not list-all)
    let agent = match state.dashboard_db.get_agent_with_stats(&agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", agent_id)})),
            )
                .into_response();
        }
        Err(e) => return internal_error(e).into_response(),
    };

    // Get core memory
    let core_memory = match agent_state.db.get_all_core_memory().await {
        Ok(entries) => entries,
        Err(e) => {
            error!(error = %e, "failed to load core memory");
            vec![]
        }
    };

    // Count core memory edits in the latest session
    let core_memory_edits_this_session = agent_state
        .db
        .count_core_memory_edits_latest_session()
        .await
        .unwrap_or(0);

    // Read soul.md from agent's home directory
    let soul_path = agent_state.home_dir.join("soul.md");
    let soul_md = tokio::fs::read_to_string(&soul_path)
        .await
        .unwrap_or_default();

    Json(AgentDetailResponse {
        id: agent.id,
        name: agent.name,
        active: agent.active,
        last_seen: agent.last_seen,
        created_at: agent.created_at,
        message_count: agent.message_count,
        core_memory,
        core_memory_edits_this_session,
        soul_md,
    })
    .into_response()
}

// ===== Agent sub-resources =====

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/agents/:id/sessions — paginated sessions for an agent.
pub async fn handle_agent_sessions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .list_sessions_paginated_with_count(
            Some(agent_id.clone()),
            None,
            None,
            None,
            per_page,
            offset,
        )
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/agents/:id/audit — paginated audit events for an agent.
pub async fn handle_agent_audit(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .list_audit_events_paginated_with_count(&agent_id, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

// ===== Sessions =====

#[derive(Debug, Deserialize)]
pub struct SessionsQuery {
    pub agent_id: Option<String>,
    pub channel_type: Option<String>,
    pub session_id: Option<String>,
    /// Filter sessions by correlated task_id (stored in session metadata JSON).
    pub task_id: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/sessions — paginated sessions with filters.
pub async fn handle_sessions_list(
    State(state): State<AppState>,
    Query(q): Query<SessionsQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .list_sessions_paginated_with_count(
            q.agent_id,
            q.channel_type,
            q.session_id,
            q.task_id,
            per_page,
            offset,
        )
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/sessions/:id — session detail.
pub async fn handle_session_detail(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_session(&session_id).await {
        Ok(Some(session)) => Json(session).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("session '{}' not found", session_id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Session Messages =====

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub id: i64,
    pub session_id: String,
    pub agent_id: String,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub created_at: String,
}

impl From<SessionMessage> for MessageResponse {
    fn from(m: SessionMessage) -> Self {
        Self {
            id: m.id,
            session_id: m.session_id,
            agent_id: m.agent_id,
            role: m.role,
            // Strip base64 image data from tool_result content to prevent multi-MB payloads
            content: strip_base64_images(&m.content),
            channel_type: m.channel_type,
            metadata: m.metadata,
            created_at: m.created_at,
        }
    }
}

/// Strip base64 image data from message content, replacing with placeholders.
/// Looks for patterns like `"data": "base64data..."` in tool_result JSON content.
fn strip_base64_images(content: &str) -> String {
    // Simple heuristic: if content contains base64 image markers, try to clean up.
    // base64 data blocks are typically very long strings following "data": " patterns.
    if content.len() < 1000 || !content.contains("base64") {
        return content.to_string();
    }

    // For very large content with base64 data, truncate to a reasonable size
    // and add a note. This prevents multi-MB payloads.
    if content.len() > 50_000 {
        let truncated = &content[..1000];
        return format!(
            "{}... [content truncated, {} bytes total — contains base64 image data]",
            truncated,
            content.len()
        );
    }

    content.to_string()
}

/// GET /api/v1/sessions/:id/messages — paginated messages for a session.
pub async fn handle_session_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .load_session_messages_paginated_with_count(&session_id, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(MessageResponse::from).collect(),
        total,
        page,
        per_page,
    })
    .into_response()
}

// ===== Team Runs =====

/// GET /api/v1/team-runs/:run_id — team run metadata.
pub async fn handle_team_run_detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.load_team_run_by_id(&run_id).await {
        Ok(Some(run)) => Json(run).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("team run '{}' not found", run_id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/team-runs/:run_id/workspace — team workspace entries.
pub async fn handle_team_workspace(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.load_team_workspace(&run_id).await {
        Ok(entries) => Json(entries).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/team-runs/:run_id/summary — enriched team run summary.
pub async fn handle_team_run_summary(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_team_run_summary(&run_id).await {
        Ok(Some(summary)) => Json(summary).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("team run '{}' not found", run_id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Tasks =====

/// Response DTO for tasks — omits sensitive/large fields, truncates previews.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskResponse {
    pub id: String,
    pub agent_id: String,
    pub label: String,
    pub trigger_type: String,
    pub action_type: String,
    pub status: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub source: Option<String>,
    pub reference_url: Option<String>,
    pub cron_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub fired_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub action_config_preview: Option<String>,
    pub result_preview: Option<String>,
    /// Task kind: `"issue"`, `"milestone"`, or `"project"`. Default `"issue"`.
    /// Added in schema v23 (mika#595).
    pub r#type: String,
}

impl From<Task> for TaskResponse {
    fn from(t: Task) -> Self {
        let action_config_preview = if t.action_config.is_empty() {
            None
        } else {
            Some(db::truncate_chars(&t.action_config, 200))
        };
        let result_preview = t.result.as_deref().map(|r| db::truncate_chars(r, 200));

        Self {
            id: t.id,
            agent_id: t.agent_id,
            label: t.label,
            trigger_type: t.trigger_type,
            action_type: t.action_type,
            status: t.status,
            team_run_id: t.team_run_id,
            parent_task_id: t.parent_task_id,
            depth: t.depth,
            source: t.source,
            reference_url: t.reference_url,
            cron_expr: t.cron_expr,
            next_fire_at: t.next_fire_at,
            fired_at: t.fired_at,
            completed_at: t.completed_at,
            created_by_session: t.created_by_session,
            created_trace_id: t.created_trace_id,
            execution_trace_id: t.execution_trace_id,
            created_at: t.created_at,
            updated_at: t.updated_at,
            action_config_preview,
            result_preview,
            r#type: t.r#type,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TasksQuery {
    pub status: Option<String>,
    pub trigger_type: Option<String>,
    pub action_type: Option<String>,
    pub agent_id: Option<String>,
    pub team_run_id: Option<String>,
    pub source: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/tasks — paginated task list with filters.
pub async fn handle_tasks_list(
    State(state): State<AppState>,
    Query(q): Query<TasksQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let team_run_id_filter = q.team_run_id.as_deref().map(|v| match v {
        "null" => TeamRunIdFilter::Null,
        "notnull" => TeamRunIdFilter::NotNull,
        specific => TeamRunIdFilter::Specific(specific.to_string()),
    });

    let filters = TaskFilters {
        status: q.status,
        trigger_type: q.trigger_type,
        action_type: q.action_type,
        agent_id: q.agent_id,
        team_run_id_filter,
        source: q.source,
    };

    let (data, total) = match state
        .dashboard_db
        .list_tasks_paginated_with_count(filters, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(TaskResponse::from).collect(),
        total,
        page,
        per_page,
    })
    .into_response()
}

/// Full task detail response with untruncated action_config and result.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskDetailResponse {
    pub id: String,
    pub agent_id: String,
    pub label: String,
    pub trigger_type: String,
    pub action_type: String,
    pub status: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub source: Option<String>,
    pub reference_url: Option<String>,
    pub cron_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub fired_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub action_config: String,
    pub result: Option<String>,
    /// Task kind: `"issue"`, `"milestone"`, or `"project"`. Default `"issue"`.
    /// Added in schema v23 (mika#595).
    pub r#type: String,
}

impl From<Task> for TaskDetailResponse {
    fn from(t: Task) -> Self {
        Self {
            id: t.id,
            agent_id: t.agent_id,
            label: t.label,
            trigger_type: t.trigger_type,
            action_type: t.action_type,
            status: t.status,
            team_run_id: t.team_run_id,
            parent_task_id: t.parent_task_id,
            depth: t.depth,
            source: t.source,
            reference_url: t.reference_url,
            cron_expr: t.cron_expr,
            next_fire_at: t.next_fire_at,
            fired_at: t.fired_at,
            completed_at: t.completed_at,
            created_by_session: t.created_by_session,
            created_trace_id: t.created_trace_id,
            execution_trace_id: t.execution_trace_id,
            created_at: t.created_at,
            updated_at: t.updated_at,
            action_config: t.action_config,
            result: t.result,
            r#type: t.r#type,
        }
    }
}

/// GET /api/v1/tasks/:id — single task detail with full action_config and result.
pub async fn handle_task_detail(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_task_unscoped(&task_id).await {
        Ok(Some(task)) => Json(TaskDetailResponse::from(task)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("task '{}' not found", task_id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/tasks/:id/children — child tasks of a parent.
pub async fn handle_task_children(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_child_tasks(&task_id).await {
        Ok(children) => Json(
            children
                .into_iter()
                .map(TaskResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Task Sessions =====

/// Response type for sessions linked to a task tree.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskSessionResponse {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub task_id: Option<String>,
    pub message_count: i64,
    pub task_label: Option<String>,
}

impl From<db::TaskSessionRow> for TaskSessionResponse {
    fn from(r: db::TaskSessionRow) -> Self {
        Self {
            id: r.id,
            agent_id: r.agent_id,
            channel_type: r.channel_type,
            started_at: r.started_at,
            ended_at: r.ended_at,
            task_id: r.task_id,
            message_count: r.message_count,
            task_label: r.task_label,
        }
    }
}

pub async fn handle_task_sessions(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state
        .dashboard_db
        .get_sessions_for_task_tree(&task_id)
        .await
    {
        Ok(rows) => {
            let sessions: Vec<TaskSessionResponse> =
                rows.into_iter().map(TaskSessionResponse::from).collect();
            Json(serde_json::json!({ "sessions": sessions })).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Team Runs List =====

#[derive(Debug, Deserialize)]
pub struct TeamRunsQuery {
    pub team_name: Option<String>,
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/team-runs — paginated team run list with filters.
pub async fn handle_team_runs_list(
    State(state): State<AppState>,
    Query(q): Query<TeamRunsQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let filters = TeamRunFilters {
        team_name: q.team_name,
        status: q.status,
        from: q.from,
        to: q.to,
    };

    let (data, total) = match state
        .dashboard_db
        .list_team_runs_paginated_with_count(filters, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

// ===== LLM Calls =====

#[derive(Debug, Deserialize)]
pub struct LlmCallsQuery {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub model: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/llm-calls — paginated LLM call list with filters.
pub async fn handle_llm_calls(
    State(state): State<AppState>,
    Query(q): Query<LlmCallsQuery>,
) -> impl IntoResponse {
    let (page, per_page, _) = resolve_pagination(q.page, q.per_page);
    let filters = db::LlmCallFilters {
        agent_id: q.agent_id,
        session_id: q.session_id,
        trace_id: q.trace_id,
        model: q.model,
        from: q.from,
        to: q.to,
    };

    let (data, total) = match state
        .dashboard_db
        .query_llm_calls(filters, page, per_page)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

// ===== Tool Calls =====

#[derive(Debug, Deserialize)]
pub struct ToolCallsQuery {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub tool_name: Option<String>,
    pub success: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/tool-calls — paginated tool call list with filters.
pub async fn handle_tool_calls(
    State(state): State<AppState>,
    Query(q): Query<ToolCallsQuery>,
) -> impl IntoResponse {
    let (page, per_page, _) = resolve_pagination(q.page, q.per_page);
    let filters = db::ToolCallFilters {
        agent_id: q.agent_id,
        session_id: q.session_id,
        trace_id: q.trace_id,
        tool_name: q.tool_name,
        success: q.success,
        from: q.from,
        to: q.to,
        keyword: None,
    };

    let (data, total) = match state
        .dashboard_db
        .query_tool_calls(filters, page, per_page)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/llm-calls/:id — single LLM call detail.
pub async fn handle_llm_call_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_llm_call_by_id(&id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("LLM call '{}' not found", id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/tool-calls/:id — single tool call detail.
pub async fn handle_tool_call_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_tool_call_by_id(&id).await {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("tool call '{}' not found", id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Trace-level LLM/Tool queries =====

/// GET /api/v1/traces/:trace_id/llm-calls
pub async fn handle_trace_llm_calls(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.query_llm_calls_by_trace(&trace_id).await {
        Ok(data) => Json(data).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

/// GET /api/v1/traces/:trace_id/tool-calls
pub async fn handle_trace_tool_calls(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state
        .dashboard_db
        .query_tool_calls_by_trace(&trace_id)
        .await
    {
        Ok(data) => Json(data).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Session-level LLM/Tool/Skills queries =====

#[derive(Debug, Deserialize)]
pub struct SessionPaginationQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/sessions/:id/llm-calls
pub async fn handle_session_llm_calls(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<SessionPaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, _) = resolve_pagination(q.page, q.per_page);
    let (data, total) = match state
        .dashboard_db
        .query_llm_calls_by_session(&session_id, page, per_page)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/sessions/:id/tool-calls
pub async fn handle_session_tool_calls(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<SessionPaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, _) = resolve_pagination(q.page, q.per_page);
    let (data, total) = match state
        .dashboard_db
        .query_tool_calls_by_session(&session_id, page, per_page)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data,
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/sessions/:id/skills — loaded skills from session metadata.
pub async fn handle_session_skills(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_session(&session_id).await {
        Ok(Some(session)) => {
            let skills_meta: serde_json::Value = session
                .metadata
                .as_deref()
                .and_then(|m| serde_json::from_str(m).ok())
                .unwrap_or(serde_json::json!({"loaded_skills": [], "skill_count": 0}));
            Json(skills_meta).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "session not found"})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::TimelineFilters;

    // ===== strip_base64_images tests =====

    #[test]
    fn strip_base64_images_short_content_unchanged() {
        let input = "Hello, world!";
        assert_eq!(strip_base64_images(input), input);
    }

    #[test]
    fn strip_base64_images_no_base64_marker_unchanged() {
        // Content over 1000 chars but no "base64" marker => returned as-is
        let input = "x".repeat(2000);
        assert_eq!(strip_base64_images(&input), input);
    }

    #[test]
    fn strip_base64_images_under_1000_with_base64_unchanged() {
        let input = "some base64 data here";
        assert_eq!(strip_base64_images(input), input);
    }

    #[test]
    fn strip_base64_images_moderate_content_with_base64_unchanged() {
        // Between 1000 and 50_000 chars with base64 marker => returned as-is
        let mut input = "x".repeat(1500);
        input.push_str(" base64 ");
        input.push_str(&"y".repeat(1000));
        assert_eq!(strip_base64_images(&input), input);
    }

    #[test]
    fn strip_base64_images_large_content_truncated() {
        // Over 50_000 chars with base64 marker => truncated
        let mut input = "HEADER_".to_string();
        input.push_str(&"a".repeat(1000));
        input.push_str(" base64 ");
        input.push_str(&"b".repeat(60_000));
        let result = strip_base64_images(&input);
        assert!(result.len() < input.len());
        assert!(result.contains("[content truncated,"));
        assert!(result.contains("bytes total"));
        assert!(result.contains("base64 image data"));
        // First 1000 chars of input should be preserved
        assert!(result.starts_with(&input[..1000]));
    }

    #[test]
    fn strip_base64_images_exactly_50000_unchanged() {
        // Exactly 50_000 chars with base64 marker => not truncated (> 50_000 required)
        let mut input = "base64".to_string();
        input.push_str(&"x".repeat(50_000 - 6));
        assert_eq!(input.len(), 50_000);
        assert_eq!(strip_base64_images(&input), input);
    }

    #[test]
    fn strip_base64_images_empty_string() {
        assert_eq!(strip_base64_images(""), "");
    }

    // ===== resolve_pagination tests =====

    #[test]
    fn resolve_pagination_defaults() {
        let (page, per_page, offset) = resolve_pagination(None, None);
        assert_eq!(page, 1);
        assert_eq!(per_page, DEFAULT_PER_PAGE);
        assert_eq!(offset, 0);
    }

    #[test]
    fn resolve_pagination_page_2() {
        let (page, per_page, offset) = resolve_pagination(Some(2), Some(10));
        assert_eq!(page, 2);
        assert_eq!(per_page, 10);
        assert_eq!(offset, 10);
    }

    #[test]
    fn resolve_pagination_page_0_clamped_to_1() {
        let (page, _per_page, offset) = resolve_pagination(Some(0), None);
        assert_eq!(page, 1);
        assert_eq!(offset, 0);
    }

    #[test]
    fn resolve_pagination_per_page_0_clamped_to_1() {
        let (_page, per_page, _offset) = resolve_pagination(None, Some(0));
        assert_eq!(per_page, 1);
    }

    #[test]
    fn resolve_pagination_per_page_over_max_clamped() {
        let (_page, per_page, _offset) = resolve_pagination(None, Some(500));
        assert_eq!(per_page, MAX_PER_PAGE);
    }

    #[test]
    fn resolve_pagination_large_page_clamped() {
        let (page, _per_page, _offset) = resolve_pagination(Some(999_999), None);
        assert_eq!(page, 100_000);
    }

    #[test]
    fn resolve_pagination_offset_calculation() {
        // page=3, per_page=25 => offset = (3-1)*25 = 50
        let (page, per_page, offset) = resolve_pagination(Some(3), Some(25));
        assert_eq!(page, 3);
        assert_eq!(per_page, 25);
        assert_eq!(offset, 50);
    }

    // ===== TimelineFilters::to_sql tests =====

    #[test]
    fn timeline_filters_empty() {
        let f = TimelineFilters::default();
        let (clause, params) = f.to_sql();
        assert_eq!(clause, "");
        assert!(params.is_empty());
    }

    #[test]
    fn timeline_filters_single_agent_id() {
        let f = TimelineFilters {
            agent_id: Some("mika".to_string()),
            ..Default::default()
        };
        let (clause, params) = f.to_sql();
        assert_eq!(clause, "WHERE agent_id = ?1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn timeline_filters_multiple_fields() {
        let f = TimelineFilters {
            agent_id: Some("mika".to_string()),
            event_type: Some("message".to_string()),
            from: Some("2026-01-01T00:00:00Z".to_string()),
            to: Some("2026-12-31T23:59:59Z".to_string()),
            ..Default::default()
        };
        let (clause, params) = f.to_sql();
        assert!(clause.starts_with("WHERE "));
        assert!(clause.contains("agent_id = ?1"));
        assert!(clause.contains("event_type = ?2"));
        assert!(clause.contains("created_at >= ?3"));
        assert!(clause.contains("created_at <= ?4"));
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn timeline_filters_all_fields() {
        let f = TimelineFilters {
            agent_id: Some("mika".to_string()),
            event_type: Some("audit".to_string()),
            trace_id: Some("abc123".to_string()),
            session_id: Some("sess-1".to_string()),
            from: Some("2026-01-01T00:00:00Z".to_string()),
            to: Some("2026-01-02T00:00:00Z".to_string()),
        };
        let (clause, params) = f.to_sql();
        assert_eq!(params.len(), 6);
        assert!(clause.contains("agent_id = ?1"));
        assert!(clause.contains("event_type = ?2"));
        assert!(clause.contains("trace_id = ?3"));
        assert!(clause.contains("session_id = ?4"));
        assert!(clause.contains("created_at >= ?5"));
        assert!(clause.contains("created_at <= ?6"));
    }

    #[test]
    fn timeline_filters_only_time_range() {
        let f = TimelineFilters {
            from: Some("2026-06-01T00:00:00Z".to_string()),
            to: Some("2026-06-02T00:00:00Z".to_string()),
            ..Default::default()
        };
        let (clause, params) = f.to_sql();
        assert_eq!(params.len(), 2);
        assert!(clause.contains("created_at >= ?1"));
        assert!(clause.contains("created_at <= ?2"));
    }

    // ===== Task DTO `type` serialization tests (issue #595) =====

    fn sample_task_with_type(t: &str) -> Task {
        Task {
            id: "task-id".to_string(),
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "sample".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            status: "pending".to_string(),
            process_id: None,
            input_context: None,
            result: None,
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-04-16T00:00:00Z".to_string(),
            updated_at: "2026-04-16T00:00:00Z".to_string(),
            fired_at: None,
            completed_at: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: t.to_string(),
        }
    }

    #[test]
    fn task_response_serializes_type_as_type_key() {
        let resp = TaskResponse::from(sample_task_with_type("milestone"));
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["type"], "milestone");
    }

    #[test]
    fn task_detail_response_serializes_type_as_type_key() {
        let resp = TaskDetailResponse::from(sample_task_with_type("project"));
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["type"], "project");
    }

    #[test]
    fn task_response_serializes_default_issue_type() {
        let resp = TaskResponse::from(sample_task_with_type("issue"));
        let json = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(json["type"], "issue");
    }
}
