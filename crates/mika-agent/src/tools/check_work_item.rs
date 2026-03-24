use std::fmt::Write;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::db::format_ts;

/// A reference parsed from a GitHub URL.
#[derive(Debug, PartialEq)]
enum GitHubRef {
    PullRequest {
        owner: String,
        repo: String,
        number: u64,
    },
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
}

/// Parse a GitHub URL into a structured reference.
///
/// Supports:
/// - `https://github.com/{owner}/{repo}/pull/{number}`
/// - `https://github.com/{owner}/{repo}/issues/{number}`
///
/// Only `github.com` is supported (no enterprise domains).
fn parse_github_ref(url: &str) -> Option<GitHubRef> {
    let url = url.trim();

    // Strip optional trailing slash and fragment/query
    let url = url.split('#').next()?;
    let url = url.split('?').next()?;
    let url = url.trim_end_matches('/');

    // Must be a github.com URL
    let path = url.strip_prefix("https://github.com/")?;

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 {
        return None;
    }

    let owner = parts[0];
    let repo = parts[1];
    let kind = parts[2];
    let number_str = parts[3];

    // Validate owner and repo are non-empty and reasonable
    if owner.is_empty() || repo.is_empty() || number_str.is_empty() {
        return None;
    }

    let number: u64 = number_str.parse().ok()?;
    if number == 0 {
        return None;
    }

    match kind {
        "pull" => Some(GitHubRef::PullRequest {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        }),
        "issues" => Some(GitHubRef::Issue {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        }),
        _ => None,
    }
}

/// Fetch GitHub PR status via the REST API.
async fn fetch_github_pr_status(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            404 => "PR not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(msg);
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;

    let state = body["state"].as_str().unwrap_or("unknown");
    let merged = body["merged"].as_bool().unwrap_or(false);
    let draft = body["draft"].as_bool().unwrap_or(false);
    let head_ref = body["head"]["ref"].as_str().unwrap_or("unknown");

    let display_state = if draft {
        "draft".to_string()
    } else if merged {
        "closed (merged)".to_string()
    } else if state == "closed" {
        "closed (not merged)".to_string()
    } else {
        state.to_string()
    };

    Ok(format!(
        "GitHub PR Status:\n  State: {display_state}\n  Branch: {head_ref}"
    ))
}

/// Fetch GitHub issue status via the REST API.
async fn fetch_github_issue_status(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            404 => "issue not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(msg);
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;

    let state = body["state"].as_str().unwrap_or("unknown");
    let state_reason = body["state_reason"].as_str();

    let display_state = match state_reason {
        Some("completed") => "closed (completed)".to_string(),
        Some("not_planned") => "closed (not planned)".to_string(),
        _ => state.to_string(),
    };

    Ok(format!("GitHub Issue Status:\n  State: {display_state}"))
}

pub struct CheckWorkItemTool;

#[async_trait]
impl Tool for CheckWorkItemTool {
    fn name(&self) -> &str {
        "check_work_item"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_work_item".to_string(),
            description: "Read work item details and check the status of any linked GitHub PR or issue. \
                Returns the work item's current status, metadata, timestamps, and — if the reference_url \
                points to a GitHub PR or issue — fetches the current state from the GitHub API. \
                Use this when the user asks about a work item's progress or wants to verify PR status \
                before deciding on a status change."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The UUID of the work item to check"
                    }
                },
                "required": ["task_id"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let task_id = input["task_id"].as_str().unwrap_or("").trim();

        if task_id.is_empty() {
            return Ok(ToolOutput::error("'task_id' is required."));
        }
        if task_id.len() > 36 {
            return Ok(ToolOutput::error(
                "'task_id' must be a valid UUID (36 characters).",
            ));
        }

        // Look up the work item (manual tasks only, agent-scoped)
        let task = match ctx.db.get_manual_task(task_id).await? {
            Some(t) => t,
            None => {
                return Ok(ToolOutput::error(format!(
                    "Work item '{task_id}' not found. Only manual (work item) tasks can be checked with this tool."
                )));
            }
        };

        // Build the base output
        let mut output = String::new();
        writeln!(output, "Work item: {}", task.id).unwrap();
        writeln!(output, "Label: {}", task.label).unwrap();
        writeln!(output, "Status: {}", task.status).unwrap();

        if let Some(ref src) = task.source {
            writeln!(output, "Source: {src}").unwrap();
        }
        if let Some(ref url) = task.reference_url {
            writeln!(output, "Reference: {url}").unwrap();
        }

        writeln!(output, "Created: {}", format_ts(&task.created_at)).unwrap();
        writeln!(output, "Updated: {}", format_ts(&task.updated_at)).unwrap();

        if let Some(ref completed) = task.completed_at {
            writeln!(output, "Completed: {}", format_ts(completed)).unwrap();
        }

        if let Some(ref metadata) = task.metadata
            && metadata != "{}"
        {
            writeln!(output, "Metadata: {metadata}").unwrap();
        }

        // GitHub enrichment
        if let Some(ref url) = task.reference_url {
            writeln!(output).unwrap();
            match parse_github_ref(url) {
                Some(GitHubRef::PullRequest {
                    ref owner,
                    ref repo,
                    number,
                }) => match ctx.github_token {
                    Some(token) => match fetch_github_pr_status(token, owner, repo, number).await {
                        Ok(status) => writeln!(output, "{status}").unwrap(),
                        Err(e) => writeln!(output, "GitHub PR status: unavailable ({e})").unwrap(),
                    },
                    None => {
                        writeln!(
                            output,
                            "GitHub PR status: not available (no token configured)"
                        )
                        .unwrap();
                    }
                },
                Some(GitHubRef::Issue {
                    ref owner,
                    ref repo,
                    number,
                }) => match ctx.github_token {
                    Some(token) => {
                        match fetch_github_issue_status(token, owner, repo, number).await {
                            Ok(status) => writeln!(output, "{status}").unwrap(),
                            Err(e) => {
                                writeln!(output, "GitHub issue status: unavailable ({e})").unwrap()
                            }
                        }
                    }
                    None => {
                        writeln!(
                            output,
                            "GitHub issue status: not available (no token configured)"
                        )
                        .unwrap();
                    }
                },
                None => {
                    writeln!(output, "Reference is not a GitHub PR or issue URL.").unwrap();
                }
            }
        }

        // Children summary
        let children = ctx.db.count_child_tasks(task_id).await?;
        if !children.is_empty() {
            let total: i64 = children.iter().map(|(_, c)| c).sum();
            let summary: Vec<String> = children
                .iter()
                .map(|(status, count)| format!("{count} {status}"))
                .collect();
            writeln!(output, "\nChildren ({total}): {}", summary.join(", ")).unwrap();
        }

        Ok(ToolOutput::success(output.trim().to_string()))
    }

    fn timeout_secs(&self) -> Option<u64> {
        // Allow extra time for GitHub API call
        Some(15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewTask;
    use crate::task_engine::types::{action_type, trigger_type};
    use crate::test_utils::test_helpers::TestHarness;

    async fn create_work_item(
        harness: &TestHarness,
        label: &str,
        reference_url: Option<&str>,
        source: Option<&str>,
    ) -> String {
        harness
            .db
            .create_task(NewTask {
                agent_id: harness.db.agent_id.clone(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: label.to_string(),
                trigger_type: trigger_type::MANUAL.to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: action_type::NONE.to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: Some("test-session".to_string()),
                created_trace_id: None,
                reference_url: reference_url.map(|s| s.to_string()),
                source: source.map(|s| s.to_string()),
                metadata: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_check_basic() {
        let harness = TestHarness::new();
        let id = create_work_item(&harness, "Test work item", None, Some("user_request")).await;
        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool
            .execute(serde_json::json!({"task_id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("Test work item"));
        assert!(result.content.contains("Status: pending"));
        assert!(result.content.contains("Source: user_request"));
    }

    #[tokio::test]
    async fn test_check_with_reference_url() {
        let harness = TestHarness::new();
        let id = create_work_item(
            &harness,
            "PR task",
            Some("https://github.com/org/repo/pull/42"),
            None,
        )
        .await;
        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool
            .execute(serde_json::json!({"task_id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result
                .content
                .contains("https://github.com/org/repo/pull/42")
        );
        // No github_token in test context → should report as not available
        assert!(
            result
                .content
                .contains("not available (no token configured)")
        );
    }

    #[tokio::test]
    async fn test_check_non_github_url() {
        let harness = TestHarness::new();
        let id = create_work_item(
            &harness,
            "External task",
            Some("https://jira.example.com/browse/PROJ-123"),
            None,
        )
        .await;
        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool
            .execute(serde_json::json!({"task_id": id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(result.content.contains("not a GitHub PR or issue URL"));
    }

    #[tokio::test]
    async fn test_check_not_found() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool
            .execute(
                serde_json::json!({"task_id": "00000000-0000-0000-0000-000000000000"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_check_empty_task_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'task_id' is required"));
    }

    #[tokio::test]
    async fn test_check_with_children() {
        let harness = TestHarness::new();
        let parent_id = create_work_item(&harness, "Parent item", None, Some("user_request")).await;

        // Create two child tasks
        for label in &["Child 1", "Child 2"] {
            harness
                .db
                .create_task(NewTask {
                    agent_id: harness.db.agent_id.clone(),
                    team_run_id: None,
                    parent_task_id: Some(parent_id.clone()),
                    depth: 1,
                    label: label.to_string(),
                    trigger_type: trigger_type::MANUAL.to_string(),
                    cron_expr: None,
                    event_source: None,
                    event_offset_secs: None,
                    condition_expr: None,
                    next_fire_at: None,
                    timeout_at: None,
                    action_type: action_type::NONE.to_string(),
                    action_config: "{}".to_string(),
                    input_context: None,
                    created_by_session: Some("test-session".to_string()),
                    created_trace_id: None,
                    reference_url: None,
                    source: None,
                    metadata: None,
                })
                .await
                .unwrap();
        }

        let ctx = harness.ctx();
        let tool = CheckWorkItemTool;

        let result = tool
            .execute(serde_json::json!({"task_id": parent_id}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result.content.contains("Children (2)"),
            "should show 2 children: {}",
            result.content
        );
    }

    // === URL parsing tests ===

    #[test]
    fn test_parse_github_pr_url() {
        assert_eq!(
            parse_github_ref("https://github.com/senara-solutions/mika/pull/257"),
            Some(GitHubRef::PullRequest {
                owner: "senara-solutions".to_string(),
                repo: "mika".to_string(),
                number: 257,
            })
        );
    }

    #[test]
    fn test_parse_github_issue_url() {
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/issues/42"),
            Some(GitHubRef::Issue {
                owner: "org".to_string(),
                repo: "repo".to_string(),
                number: 42,
            })
        );
    }

    #[test]
    fn test_parse_github_url_with_trailing_slash() {
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/pull/1/"),
            Some(GitHubRef::PullRequest {
                owner: "org".to_string(),
                repo: "repo".to_string(),
                number: 1,
            })
        );
    }

    #[test]
    fn test_parse_github_url_with_fragment() {
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/issues/5#issuecomment-123"),
            Some(GitHubRef::Issue {
                owner: "org".to_string(),
                repo: "repo".to_string(),
                number: 5,
            })
        );
    }

    #[test]
    fn test_parse_non_github_url() {
        assert_eq!(
            parse_github_ref("https://jira.example.com/browse/PROJ-123"),
            None
        );
    }

    #[test]
    fn test_parse_github_non_pr_issue_path() {
        // Valid github.com URL but not a PR or issue
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/tree/main"),
            None
        );
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/commits/main"),
            None
        );
    }

    #[test]
    fn test_parse_github_invalid_number() {
        assert_eq!(
            parse_github_ref("https://github.com/org/repo/pull/abc"),
            None
        );
        assert_eq!(parse_github_ref("https://github.com/org/repo/pull/0"), None);
    }

    #[test]
    fn test_parse_empty_url() {
        assert_eq!(parse_github_ref(""), None);
        assert_eq!(parse_github_ref("   "), None);
    }

    #[test]
    fn test_parse_http_url_rejected() {
        // Only HTTPS supported
        assert_eq!(parse_github_ref("http://github.com/org/repo/pull/1"), None);
    }
}
