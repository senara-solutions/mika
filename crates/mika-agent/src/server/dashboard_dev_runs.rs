//! Dashboard API handlers for Dev Runs — claude-pilot tasks.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::warn;
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

// ===== GitHub Proxy Types =====

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GitHubIssueResponse {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<GitHubLabel>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GitHubPullResponse {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub merged: bool,
    pub reviews: Vec<GitHubReview>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GitHubReview {
    pub user: String,
    pub state: String,
    pub body: Option<String>,
    pub submitted_at: Option<String>,
}

/// GitHub API response types (internal — for deserializing upstream responses).
mod github_api {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Issue {
        pub title: String,
        pub body: Option<String>,
        pub state: String,
        pub labels: Vec<Label>,
    }

    #[derive(Deserialize)]
    pub struct Label {
        pub name: String,
        pub color: String,
    }

    #[derive(Deserialize)]
    pub struct Pull {
        pub title: String,
        pub body: Option<String>,
        pub state: String,
        pub additions: u32,
        pub deletions: u32,
        pub changed_files: u32,
        #[serde(default)]
        pub merged: bool,
    }

    #[derive(Deserialize)]
    pub struct Review {
        pub user: User,
        pub state: String,
        pub body: Option<String>,
        pub submitted_at: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct User {
        pub login: String,
    }
}

/// Shared helper to make authenticated GitHub API requests.
async fn github_get<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    token: &str,
    url: &str,
) -> Result<T, (StatusCode, String)> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "mika-dashboard")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| {
            warn!("GitHub API request failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                "GitHub API unreachable".to_string(),
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "GitHub authentication failed".to_string(),
            403 => "GitHub rate limit exceeded".to_string(),
            404 => "Not found".to_string(),
            _ => format!("GitHub API returned {status}"),
        };
        let code = match status.as_u16() {
            404 => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_GATEWAY,
        };
        return Err((code, msg));
    }

    response.json::<T>().await.map_err(|e| {
        warn!("Failed to parse GitHub API response: {e}");
        (
            StatusCode::BAD_GATEWAY,
            "Failed to parse GitHub response".to_string(),
        )
    })
}

// ===== GitHub Proxy Helpers =====

/// Validate GitHub owner/repo path segments to prevent SSRF-like probing.
/// Only allows alphanumeric, hyphens, underscores, and dots.
fn is_valid_github_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

// ===== GitHub Proxy Handlers =====

/// GET /api/v1/github/issues/{owner}/{repo}/{number} — proxy to GitHub Issues API.
pub async fn handle_github_issue(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, u32)>,
) -> impl IntoResponse {
    if !is_valid_github_name(&owner) || !is_valid_github_name(&repo) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid owner or repo name"})),
        )
            .into_response();
    }

    let Some(token) = &state.github_token else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GitHub token not configured"})),
        )
            .into_response();
    };

    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}");
    match github_get::<github_api::Issue>(&state.http_client, token, &url).await {
        Ok(issue) => Json(GitHubIssueResponse {
            title: issue.title,
            body: issue.body,
            state: issue.state,
            labels: issue
                .labels
                .into_iter()
                .map(|l| GitHubLabel {
                    name: l.name,
                    color: l.color,
                })
                .collect(),
        })
        .into_response(),
        Err((code, msg)) => (code, Json(serde_json::json!({"error": msg}))).into_response(),
    }
}

/// GET /api/v1/github/pulls/{owner}/{repo}/{number} — proxy to GitHub Pulls API.
///
/// Also fetches reviews via a second request to `/pulls/{number}/reviews`.
pub async fn handle_github_pull(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, u32)>,
) -> impl IntoResponse {
    if !is_valid_github_name(&owner) || !is_valid_github_name(&repo) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid owner or repo name"})),
        )
            .into_response();
    }

    let Some(token) = &state.github_token else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GitHub token not configured"})),
        )
            .into_response();
    };

    let pull_url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let reviews_url =
        format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews?per_page=20");

    // Fetch PR and reviews in parallel.
    let (pull_result, reviews_result) = tokio::join!(
        github_get::<github_api::Pull>(&state.http_client, token, &pull_url),
        github_get::<Vec<github_api::Review>>(&state.http_client, token, &reviews_url),
    );

    let pull = match pull_result {
        Ok(p) => p,
        Err((code, msg)) => {
            return (code, Json(serde_json::json!({"error": msg}))).into_response();
        }
    };

    // Reviews are best-effort — if they fail, return empty list.
    let reviews = reviews_result.unwrap_or_else(|e| {
        warn!("Failed to fetch PR reviews: {}", e.1);
        Vec::new()
    });

    Json(GitHubPullResponse {
        title: pull.title,
        body: pull.body,
        state: pull.state,
        additions: pull.additions,
        deletions: pull.deletions,
        changed_files: pull.changed_files,
        merged: pull.merged,
        reviews: reviews
            .into_iter()
            .map(|r| GitHubReview {
                user: r.user.login,
                state: r.state,
                body: r.body,
                submitted_at: r.submitted_at,
            })
            .collect(),
    })
    .into_response()
}
