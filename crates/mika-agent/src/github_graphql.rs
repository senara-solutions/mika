//! Shared helpers for querying GitHub's GraphQL API.
//!
//! Extracted from `skills::executor` so that both the blocked-by dispatch guard
//! and the `resolve_issue_order` tool can reuse the same HTTP + parsing logic.

use std::time::Duration;

/// Best-effort PR summary returned by [`fetch_pr_summary`] (mika#920).
///
/// Returned by GitHub REST `/repos/{owner}/{repo}/pulls/{number}` plus
/// `/repos/{owner}/{repo}/pulls/{number}/reviews`. All fields are optional —
/// the dispatch guard rejects based on the DB-level `pr_url` presence alone,
/// so any API failure or missing field degrades gracefully (omit + warn).
#[derive(Debug, Clone)]
pub(crate) struct PrSummary {
    /// `"open"`, `"closed"`, or `"merged"` (GitHub returns `state="closed"` + `merged=true`).
    pub state: Option<String>,
    /// `mergeable_state` from REST: `"clean"`, `"blocked"`, `"behind"`, `"dirty"`, etc.
    pub merge_state: Option<String>,
    /// Latest `VERDICT:` line parsed from the most recent mika-qa review body.
    pub latest_verdict: Option<String>,
}

/// Fetch a best-effort PR summary for the dispatch-readiness guard (mika#920).
///
/// Makes two REST calls: one for PR state + mergeable_state, one for the
/// reviews list. Returns `Ok(PrSummary)` with whatever fields could be parsed
/// from successful responses; returns `Err` only on transport/JSON failure of
/// the primary PR request. The caller treats the whole API call as
/// best-effort: on `Err`, omit the enriched fields and still reject the
/// dispatch based on `pr_url` presence in DB metadata.
pub(crate) async fn fetch_pr_summary(
    token: &str,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Result<PrSummary, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let pr_url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}");
    let pr_resp = client
        .get(&pr_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub PR request failed: {e}"))?;

    let status = pr_resp.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            404 => "PR not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub PR API error: {msg}"));
    }

    let pr_body: serde_json::Value = pr_resp
        .json()
        .await
        .map_err(|e| format!("failed to parse PR response: {e}"))?;

    let merged = pr_body
        .get("merged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let raw_state = pr_body.get("state").and_then(|v| v.as_str());
    let state = match (raw_state, merged) {
        (Some("closed"), true) => Some("merged".to_string()),
        (Some(s), _) => Some(s.to_string()),
        (None, _) => None,
    };
    let merge_state = pr_body
        .get("mergeable_state")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Reviews: best-effort secondary fetch. On any failure, omit verdict.
    let reviews_url =
        format!("https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}/reviews");
    let latest_verdict = match client
        .get(&reviews_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => extract_latest_verdict(&body),
            Err(_) => None,
        },
        _ => None,
    };

    Ok(PrSummary {
        state,
        merge_state,
        latest_verdict,
    })
}

/// Parse the `VERDICT:` value from the most recently submitted review body.
///
/// GitHub returns reviews in chronological order; we walk in reverse to find
/// the latest review whose body contains a `VERDICT:` line. Returns the
/// trimmed value after `VERDICT:` (e.g., `"block[ac]"`, `"pass"`, `"hold[review]"`).
pub(crate) fn extract_latest_verdict(reviews_body: &serde_json::Value) -> Option<String> {
    let arr = reviews_body.as_array()?;
    for review in arr.iter().rev() {
        let body = review.get("body").and_then(|v| v.as_str())?;
        for line in body.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("VERDICT:") {
                let verdict = rest.trim();
                if !verdict.is_empty() {
                    return Some(verdict.to_string());
                }
            }
        }
    }
    None
}

/// Parse `(owner, repo, pr_number)` from a GitHub PR URL.
///
/// Expects `https://github.com/{owner}/{repo}/pull/{number}` shape; returns
/// `None` on any structural mismatch (also accepts `/pulls/` defensively).
pub(crate) fn parse_pr_url(pr_url: &str) -> Option<(String, String, u64)> {
    let path = pr_url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 4 {
        return None;
    }
    let owner = parts[0];
    let repo = parts[1];
    if !matches!(parts[2], "pull" | "pulls") {
        return None;
    }
    let number = parts[3].parse::<u64>().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string(), number))
}

/// Fetch the body of a GitHub issue via the REST API.
///
/// Returns the raw body text on success. Used by the dispatch-readiness
/// grooming-marker check (mika#919).
///
/// Reuses the same auth/header/timeout shape as `tools::check_task::
/// github_get` for consistency with the existing GitHub HTTP layer.
pub(crate) async fn fetch_issue_body(
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
            404 => "not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub API error: {msg}"));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;

    body.get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "issue body field missing or null".to_string())
}

/// Query GitHub's GraphQL API for `blockedBy` edges on an issue.
///
/// Returns the issue numbers of any open (non-CLOSED) blockers. Returns an empty
/// vec when there are no open blockers. Returns `Err` on API or network failures.
pub(crate) async fn fetch_open_blockers(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<u64>, String> {
    let query_str = "query($owner:String!,$repo:String!,$number:Int!) { \
         repository(owner:$owner,name:$repo) { \
         issue(number:$number) { \
         blockedBy(first:100) { nodes { number state } } \
         } } }";
    let body = serde_json::json!({
        "query": query_str,
        "variables": {
            "owner": owner,
            "repo": repo,
            "number": number,
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("GitHub GraphQL request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub GraphQL API error: {msg}"));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse GraphQL response: {e}"))?;

    // Check for GraphQL-level errors
    if let Some(errors) = body.get("errors") {
        let msg = errors
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown GraphQL error");
        return Err(format!("GitHub GraphQL error: {msg}"));
    }

    Ok(extract_open_blocker_numbers(&body))
}

/// Extract open (non-CLOSED) blocker issue numbers from a GitHub GraphQL response.
///
/// Navigates `data.repository.issue.blockedBy.nodes` and returns issue numbers
/// where `state != "CLOSED"`. Returns an empty vec when the path is absent (e.g., repo
/// does not support sub-issues).
pub(crate) fn extract_open_blocker_numbers(body: &serde_json::Value) -> Vec<u64> {
    let nodes = body
        .pointer("/data/repository/issue/blockedBy/nodes")
        .and_then(|n| n.as_array());

    let Some(nodes) = nodes else {
        return vec![];
    };

    nodes
        .iter()
        .filter(|node| {
            node.get("state")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s != "CLOSED")
        })
        .filter_map(|node| node.get("number").and_then(|n| n.as_u64()))
        .collect()
}

// ---------------------------------------------------------------------------
// Phase label helpers (mika#1153)
// ---------------------------------------------------------------------------

/// Extract the phase number from a list of label names.
///
/// Looks for labels matching `phase:N` (1-indexed). Returns `None` if no
/// phase label is found. Logs a warning and returns the first match if
/// multiple phase labels are present.
pub(crate) fn parse_phase_label(labels: &[String]) -> Option<u32> {
    let mut found: Option<u32> = None;
    for label in labels {
        if let Some(n_str) = label.strip_prefix("phase:")
            && let Ok(n) = n_str.parse::<u32>()
        {
            if n == 0 {
                continue; // phases are 1-indexed
            }
            if let Some(first) = found {
                tracing::warn!(
                    first = first,
                    duplicate = n,
                    "issue has multiple phase labels, using first"
                );
                return found;
            }
            found = Some(n);
        }
    }
    found
}

/// Lightweight representation of a milestone sub-issue from the GitHub REST API.
#[derive(Debug, Clone)]
pub(crate) struct MilestoneIssue {
    pub number: u64,
    pub state: String,
    pub labels: Vec<String>,
}

/// Fetch all labels for a specific issue via GitHub REST API.
///
/// Returns a vector of label name strings. 10s timeout, Bearer auth.
pub(crate) async fn fetch_issue_labels(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/labels");

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
            404 => "not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub API error: {msg}"));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse labels response: {e}"))?;

    let labels = body
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(labels)
}

/// Fetch issues in a milestone by state via GitHub REST API.
///
/// Returns all issues with their labels. Paginates up to 100 (single page).
/// `state` should be "open", "closed", or "all".
pub(crate) async fn fetch_milestone_issues_by_state(
    token: &str,
    owner: &str,
    repo: &str,
    milestone_number: u64,
    state: &str,
) -> Result<Vec<MilestoneIssue>, String> {
    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/issues?milestone={milestone_number}&state={state}&per_page=100"
    );

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
            404 => "not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub API error: {msg}"));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse milestone issues response: {e}"))?;

    let issues = body
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|issue| {
                    let number = issue.get("number")?.as_u64()?;
                    let state = issue.get("state")?.as_str()?.to_string();
                    let labels = issue
                        .get("labels")
                        .and_then(|l| l.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|l| {
                                    l.get("name").and_then(|n| n.as_str()).map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(MilestoneIssue {
                        number,
                        state,
                        labels,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(issues)
}

/// Fetch the milestone number associated with a GitHub issue.
///
/// Makes a GET request to `/repos/{owner}/{repo}/issues/{number}` and extracts
/// the `milestone.number` field. Returns `None` if the issue has no milestone.
pub(crate) async fn fetch_issue_milestone_number(
    token: &str,
    owner: &str,
    repo: &str,
    issue_number: u64,
) -> Result<u64, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{issue_number}");

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
            404 => "not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub API error: {msg}"));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse issue response: {e}"))?;

    body.get("milestone")
        .and_then(|m| m.get("number"))
        .and_then(|n| n.as_u64())
        .ok_or_else(|| "issue has no milestone".to_string())
}

/// Add a label to a GitHub issue via REST API.
///
/// POST /repos/{owner}/{repo}/issues/{number}/labels — idempotent (adding an
/// existing label is a no-op). Returns `Ok(())` on success.
pub(crate) async fn add_label_to_issue(
    token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    label: &str,
) -> Result<(), String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/labels");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let body = serde_json::json!({ "labels": [label] });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            404 => "not found or not accessible".to_string(),
            410 => "issue is gone (transferred or deleted)".to_string(),
            422 => "validation failed (label may not exist)".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("GitHub label API error for issue #{number}: {msg}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blockers_all_closed() {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "issue": {
                        "blockedBy": {
                            "nodes": [
                                {"number": 100, "state": "CLOSED"},
                                {"number": 101, "state": "CLOSED"}
                            ]
                        }
                    }
                }
            }
        });
        assert_eq!(extract_open_blocker_numbers(&body), Vec::<u64>::new());
    }

    #[test]
    fn test_parse_blockers_some_open() {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "issue": {
                        "blockedBy": {
                            "nodes": [
                                {"number": 689, "state": "OPEN"},
                                {"number": 690, "state": "CLOSED"},
                                {"number": 691, "state": "OPEN"}
                            ]
                        }
                    }
                }
            }
        });
        assert_eq!(extract_open_blocker_numbers(&body), vec![689, 691]);
    }

    #[test]
    fn test_parse_blockers_empty_nodes() {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "issue": {
                        "blockedBy": {
                            "nodes": []
                        }
                    }
                }
            }
        });
        assert_eq!(extract_open_blocker_numbers(&body), Vec::<u64>::new());
    }

    #[test]
    fn test_parse_blockers_missing_blocked_by_field() {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "issue": {}
                }
            }
        });
        assert_eq!(extract_open_blocker_numbers(&body), Vec::<u64>::new());
    }

    #[test]
    fn test_parse_blockers_no_data() {
        let body = serde_json::json!({"errors": [{"message": "not found"}]});
        assert_eq!(extract_open_blocker_numbers(&body), Vec::<u64>::new());
    }

    // -- PR URL parsing (mika#920) --

    #[test]
    fn test_parse_pr_url_canonical_shape() {
        let parsed = parse_pr_url("https://github.com/senara-solutions/mika/pull/915");
        assert_eq!(
            parsed,
            Some(("senara-solutions".to_string(), "mika".to_string(), 915))
        );
    }

    #[test]
    fn test_parse_pr_url_with_trailing_path() {
        // GitHub adds `/files`, `/commits`, etc. — should still parse the head.
        let parsed = parse_pr_url("https://github.com/senara-solutions/mika/pull/915/files");
        assert_eq!(
            parsed,
            Some(("senara-solutions".to_string(), "mika".to_string(), 915))
        );
    }

    #[test]
    fn test_parse_pr_url_rejects_issue_url() {
        let parsed = parse_pr_url("https://github.com/senara-solutions/mika/issues/920");
        assert_eq!(parsed, None);
    }

    #[test]
    fn test_parse_pr_url_rejects_garbage() {
        assert_eq!(parse_pr_url(""), None);
        assert_eq!(parse_pr_url("https://example.com/foo/bar/pull/1"), None);
        assert_eq!(
            parse_pr_url("https://github.com/senara-solutions/mika/pull/notanumber"),
            None
        );
    }

    // -- Verdict extraction from reviews list (mika#920) --

    #[test]
    fn test_extract_latest_verdict_returns_most_recent() {
        let body = serde_json::json!([
            { "body": "First review.\nVERDICT: pass\n" },
            { "body": "Second pass.\nVERDICT: block[ac]\n" },
        ]);
        assert_eq!(extract_latest_verdict(&body), Some("block[ac]".to_string()));
    }

    #[test]
    fn test_extract_latest_verdict_skips_reviews_without_verdict() {
        let body = serde_json::json!([
            { "body": "VERDICT: pass" },
            { "body": "Just a comment, no verdict." },
        ]);
        assert_eq!(extract_latest_verdict(&body), Some("pass".to_string()));
    }

    #[test]
    fn test_extract_latest_verdict_none_when_no_verdict() {
        let body = serde_json::json!([
            { "body": "Just a comment." },
            { "body": "Another comment." },
        ]);
        assert_eq!(extract_latest_verdict(&body), None);
    }

    #[test]
    fn test_extract_latest_verdict_empty_array() {
        let body = serde_json::json!([]);
        assert_eq!(extract_latest_verdict(&body), None);
    }

    // -- Phase label parsing (mika#1153) --

    #[test]
    fn test_parse_phase_label_found() {
        let labels = vec![
            "bug".to_string(),
            "phase:2".to_string(),
            "p1-important".to_string(),
        ];
        assert_eq!(parse_phase_label(&labels), Some(2));
    }

    #[test]
    fn test_parse_phase_label_phase_one() {
        let labels = vec!["phase:1".to_string()];
        assert_eq!(parse_phase_label(&labels), Some(1));
    }

    #[test]
    fn test_parse_phase_label_empty() {
        let labels: Vec<String> = vec![];
        assert_eq!(parse_phase_label(&labels), None);
    }

    #[test]
    fn test_parse_phase_label_no_phase() {
        let labels = vec!["enhancement".to_string(), "agent-core".to_string()];
        assert_eq!(parse_phase_label(&labels), None);
    }

    #[test]
    fn test_parse_phase_label_zero_rejected() {
        let labels = vec!["phase:0".to_string()];
        assert_eq!(parse_phase_label(&labels), None);
    }

    #[test]
    fn test_parse_phase_label_non_numeric() {
        let labels = vec!["phase:abc".to_string()];
        assert_eq!(parse_phase_label(&labels), None);
    }

    #[test]
    fn test_parse_phase_label_multiple_first_wins() {
        let labels = vec!["phase:1".to_string(), "phase:2".to_string()];
        assert_eq!(parse_phase_label(&labels), Some(1));
    }
}
