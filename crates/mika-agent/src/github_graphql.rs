//! Shared helpers for querying GitHub's GraphQL API.
//!
//! Extracted from `skills::executor` so that both the blocked-by dispatch guard
//! and the `resolve_issue_order` tool can reuse the same HTTP + parsing logic.

use std::time::Duration;

/// Query GitHub's GraphQL API for `blockedByIssues` edges on an issue.
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
         blockedByIssues(first:100) { nodes { number state } } \
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
/// Navigates `data.repository.issue.blockedByIssues.nodes` and returns issue numbers
/// where `state != "CLOSED"`. Returns an empty vec when the path is absent (e.g., repo
/// does not support sub-issues).
pub(crate) fn extract_open_blocker_numbers(body: &serde_json::Value) -> Vec<u64> {
    let nodes = body
        .pointer("/data/repository/issue/blockedByIssues/nodes")
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
                        "blockedByIssues": {
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
                        "blockedByIssues": {
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
                        "blockedByIssues": {
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
}
