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

use crate::db::{CoreMemoryEntry, SessionMessage, TimelineFilters};

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
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub message_count: i64,
    pub core_memory: Vec<CoreMemoryEntry>,
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

/// GET /api/v1/agents/:id/sessions — paginated sessions for an agent.
pub async fn handle_agent_sessions(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .list_sessions_paginated_with_count(Some(agent_id.clone()), None, per_page, offset)
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
        .list_sessions_paginated_with_count(q.agent_id, q.channel_type, per_page, offset)
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
    pub created_at: i64,
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
            from: Some(1000),
            to: Some(2000),
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
            from: Some(100),
            to: Some(200),
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
            from: Some(500),
            to: Some(600),
            ..Default::default()
        };
        let (clause, params) = f.to_sql();
        assert_eq!(params.len(), 2);
        assert!(clause.contains("created_at >= ?1"));
        assert!(clause.contains("created_at <= ?2"));
    }
}
