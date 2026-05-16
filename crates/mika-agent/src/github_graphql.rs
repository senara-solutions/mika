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
}
