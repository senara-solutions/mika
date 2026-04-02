//! Dashboard API handlers for Dev Runs — claude-pilot work items.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::Task;

use super::dashboard::{PaginatedResponse, internal_error, resolve_pagination};
use super::state::AppState;

// ===== Types =====

#[derive(Debug, Serialize, ToSchema)]
pub struct DevRunResponse {
    pub id: String,
    pub agent_id: String,
    pub source: Option<String>,
    pub label: String,
    pub status: String,
    pub reference_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    // Extracted from metadata.claude_pilot:
    pub branch: Option<String>,
    pub repo: Option<String>,
    pub pr_number: Option<u32>,
    pub pr_url: Option<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub turns: Option<u32>,
    pub session_id: Option<String>,
}

impl From<Task> for DevRunResponse {
    fn from(t: Task) -> Self {
        let (branch, repo, pr_number, pr_url, cost_usd, duration_ms, turns, session_id) = t
            .metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("claude_pilot").cloned())
            .map(|cp| {
                (
                    cp.get("branch")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    cp.get("repo")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    cp.get("pr_number")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32),
                    cp.get("pr_url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    cp.get("cost_usd").and_then(|v| {
                        v.as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                    }),
                    cp.get("duration_ms").and_then(|v| v.as_u64()),
                    cp.get("turns").and_then(|v| v.as_u64()).map(|n| n as u32),
                    cp.get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                )
            })
            .unwrap_or_default();

        Self {
            id: t.id,
            agent_id: t.agent_id,
            source: t.source,
            label: t.label,
            status: t.status,
            reference_url: t.reference_url,
            created_at: t.created_at,
            updated_at: t.updated_at,
            completed_at: t.completed_at,
            branch,
            repo,
            pr_number,
            pr_url,
            cost_usd,
            duration_ms,
            turns,
            session_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DevRunsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

// ===== Handlers =====

/// GET /api/v1/dev-runs — paginated list of dev runs.
pub async fn handle_dev_runs_list(
    State(state): State<AppState>,
    Query(q): Query<DevRunsQuery>,
) -> impl IntoResponse {
    let (page, per_page, offset) = resolve_pagination(q.page, q.per_page);

    let (data, total) = match state
        .dashboard_db
        .list_dev_runs_paginated_with_count(q.status, per_page, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => return internal_error(e).into_response(),
    };

    Json(PaginatedResponse {
        data: data.into_iter().map(DevRunResponse::from).collect(),
        total,
        page,
        per_page,
    })
    .into_response()
}

/// GET /api/v1/dev-runs/:task_id — single dev run detail.
pub async fn handle_dev_run_detail(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match state.dashboard_db.get_dev_run(&task_id).await {
        Ok(Some(task)) => Json(DevRunResponse::from(task)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("dev run '{}' not found", task_id)})),
        )
            .into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}
