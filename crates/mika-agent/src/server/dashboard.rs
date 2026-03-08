//! Dashboard API handlers — read-only endpoints for the observability dashboard.

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
    AgentWithStats, AuditEvent, CoreMemoryEntry, Session, SessionMessage, SessionWithStats,
    TimelineFilters, TimelineRow,
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

fn resolve_pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, u32) {
    let page = page.unwrap_or(1).clamp(1, 100_000);
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let offset = (page - 1).saturating_mul(per_page);
    (page, per_page, offset)
}

fn internal_error(e: anyhow::Error) -> impl IntoResponse {
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
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimelineRowResponse {
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub event_type: String,
    pub event_subtype: String,
    pub summary: Option<String>,
    pub created_at: i64,
}

impl From<TimelineRow> for TimelineRowResponse {
    fn from(r: TimelineRow) -> Self {
        Self {
            trace_id: r.trace_id,
            session_id: r.session_id,
            agent_id: r.agent_id,
            event_type: r.event_type,
            event_subtype: r.event_subtype,
            summary: r.summary,
            created_at: r.created_at,
        }
    }
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
    let count_filters = filters.clone();

    let data = match state
        .dashboard_db
        .query_timeline(filters, per_page, offset)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return internal_error(e).into_response(),
    };

    let total = match state.dashboard_db.query_timeline_count(count_filters).await {
        Ok(n) => n,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(TimelineRowResponse::from).collect(),
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
        Ok(rows) => Json(
            rows.into_iter()
                .map(TimelineRowResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

// ===== Agents =====

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub message_count: i64,
}

impl From<AgentWithStats> for AgentResponse {
    fn from(a: AgentWithStats) -> Self {
        Self {
            id: a.id,
            name: a.name,
            active: a.active,
            last_seen: a.last_seen,
            created_at: a.created_at,
            message_count: a.message_count,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentDetailResponse {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub message_count: i64,
    pub core_memory: Vec<CoreMemoryResponse>,
    pub soul_md: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CoreMemoryResponse {
    pub key: String,
    pub value: String,
    pub token_count: i32,
    pub updated_at: String,
}

impl From<CoreMemoryEntry> for CoreMemoryResponse {
    fn from(e: CoreMemoryEntry) -> Self {
        Self {
            key: e.key,
            value: e.value,
            token_count: e.token_count,
            updated_at: e.updated_at,
        }
    }
}

/// GET /api/v1/agents — list all agents with stats.
pub async fn handle_agents_list(State(state): State<AppState>) -> impl IntoResponse {
    match state.dashboard_db.list_agents_with_stats().await {
        Ok(agents) => Json(
            agents
                .into_iter()
                .map(AgentResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
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
        Ok(entries) => entries.into_iter().map(CoreMemoryResponse::from).collect(),
        Err(e) => {
            error!(error = %e, "failed to load core memory");
            vec![]
        }
    };

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

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata: Option<String>,
    pub message_count: i64,
}

impl From<SessionWithStats> for SessionResponse {
    fn from(s: SessionWithStats) -> Self {
        Self {
            id: s.id,
            agent_id: s.agent_id,
            channel_type: s.channel_type,
            started_at: s.started_at,
            ended_at: s.ended_at,
            metadata: s.metadata,
            message_count: s.message_count,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEventResponse {
    pub id: i64,
    pub session_id: String,
    pub tool_name: String,
    pub target_key: String,
    pub before_value: Option<String>,
    pub after_value: String,
    pub reasoning: Option<String>,
    pub created_at: String,
}

impl From<AuditEvent> for AuditEventResponse {
    fn from(e: AuditEvent) -> Self {
        Self {
            id: e.id,
            session_id: e.session_id,
            tool_name: e.tool_name,
            target_key: e.target_key,
            before_value: e.before_value,
            after_value: e.after_value,
            reasoning: e.reasoning,
            created_at: e.created_at,
        }
    }
}

/// GET /api/v1/agents/:id/sessions — paginated sessions for an agent.
pub async fn handle_agent_sessions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let data = match state
        .dashboard_db
        .list_sessions_paginated(Some(agent_id.clone()), None, per_page, offset)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return internal_error(e).into_response(),
    };

    let total = match state
        .dashboard_db
        .count_sessions(Some(agent_id), None)
        .await
    {
        Ok(n) => n,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(SessionResponse::from).collect(),
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

    let data = match state
        .dashboard_db
        .list_audit_events_paginated(&agent_id, per_page, offset)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return internal_error(e).into_response(),
    };

    let total = match state.dashboard_db.count_audit_events(&agent_id).await {
        Ok(n) => n,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(AuditEventResponse::from).collect(),
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
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// GET /api/v1/sessions — paginated sessions with filters.
pub async fn handle_sessions_list(
    State(state): State<AppState>,
    Query(q): Query<SessionsQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let data = match state
        .dashboard_db
        .list_sessions_paginated(q.agent_id.clone(), q.channel_type.clone(), per_page, offset)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return internal_error(e).into_response(),
    };

    let total = match state
        .dashboard_db
        .count_sessions(q.agent_id, q.channel_type)
        .await
    {
        Ok(n) => n,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(SessionResponse::from).collect(),
        total,
        page,
        per_page,
    })
    .into_response()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionDetailResponse {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata: Option<String>,
}

impl From<Session> for SessionDetailResponse {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            agent_id: s.agent_id,
            channel_type: s.channel_type,
            started_at: s.started_at,
            ended_at: s.ended_at,
            metadata: s.metadata,
        }
    }
}

/// GET /api/v1/sessions/:id — session detail.
pub async fn handle_session_detail(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_session(&session_id).await {
        Ok(Some(session)) => Json(SessionDetailResponse::from(session)).into_response(),
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
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub created_at: i64,
}

impl From<SessionMessage> for MessageResponse {
    fn from(m: SessionMessage) -> Self {
        Self {
            id: m.id,
            session_id: m.session_id,
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

    let data = match state
        .dashboard_db
        .load_session_messages_paginated(&session_id, per_page, offset)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return internal_error(e).into_response(),
    };

    let total = match state.dashboard_db.count_session_messages(&session_id).await {
        Ok(n) => n,
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
