use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use tracing::warn;

use super::{Tool, ToolContext, ToolOutput};
use crate::github_graphql::fetch_open_blockers;

pub struct ResolveIssueOrderTool;

/// Result of a topological sort over issue numbers.
#[derive(Debug)]
struct TopoSortResult {
    /// Issues in dependency-safe execution order.
    sorted: Vec<u64>,
    /// Intra-list blocked-by edges: key is blocked by values.
    edges: HashMap<u64, Vec<u64>>,
    /// Blockers that are outside the input issue list.
    external_blockers: HashMap<u64, Vec<u64>>,
    /// Issues involved in a dependency cycle (if any).
    cycle: Option<Vec<u64>>,
}

/// Pure topological sort using Kahn's algorithm with ascending tiebreaker.
///
/// `issues` is the set of issue numbers to sort.
/// `blocked_by` maps each issue to the list of issues that block it (may include
/// issues outside the input set — those are tracked as external blockers).
fn topological_sort(issues: &[u64], blocked_by: &HashMap<u64, Vec<u64>>) -> TopoSortResult {
    let issue_set: HashSet<u64> = issues.iter().copied().collect();

    // Separate intra-list edges from external blockers.
    let mut edges: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut external_blockers: HashMap<u64, Vec<u64>> = HashMap::new();

    for &issue in issues {
        if let Some(blockers) = blocked_by.get(&issue) {
            let mut intra = Vec::new();
            let mut external = Vec::new();
            for &b in blockers {
                if issue_set.contains(&b) {
                    intra.push(b);
                } else {
                    external.push(b);
                }
            }
            if !intra.is_empty() {
                intra.sort_unstable();
                edges.insert(issue, intra);
            }
            if !external.is_empty() {
                external.sort_unstable();
                external_blockers.insert(issue, external);
            }
        }
    }

    // Build in-degree map and adjacency list (blocker -> dependents).
    let mut in_degree: HashMap<u64, usize> = HashMap::new();
    let mut dependents: HashMap<u64, Vec<u64>> = HashMap::new();

    for &issue in issues {
        in_degree.entry(issue).or_insert(0);
    }
    for (&issue, blockers) in &edges {
        *in_degree.entry(issue).or_insert(0) += blockers.len();
        for &b in blockers {
            dependents.entry(b).or_default().push(issue);
        }
    }

    // Kahn's algorithm with min-heap for ascending tiebreaker.
    let mut heap: BinaryHeap<Reverse<u64>> = BinaryHeap::new();
    for (&issue, &deg) in &in_degree {
        if deg == 0 {
            heap.push(Reverse(issue));
        }
    }

    let mut sorted = Vec::with_capacity(issues.len());
    while let Some(Reverse(current)) = heap.pop() {
        sorted.push(current);
        if let Some(deps) = dependents.get(&current) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(&dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        heap.push(Reverse(dep));
                    }
                }
            }
        }
    }

    // If sorted has fewer items than input, the remaining nodes form a cycle.
    let cycle = if sorted.len() < issues.len() {
        let sorted_set: HashSet<u64> = sorted.iter().copied().collect();
        let mut cycle_nodes: Vec<u64> = issues
            .iter()
            .copied()
            .filter(|i| !sorted_set.contains(i))
            .collect();
        cycle_nodes.sort_unstable();
        // Only the cycle nodes are returned; sorted contains the non-cycle prefix.
        // Clear sorted since partial order is unreliable when cycles exist.
        sorted.clear();
        Some(cycle_nodes)
    } else {
        None
    };

    TopoSortResult {
        sorted,
        edges,
        external_blockers,
        cycle,
    }
}

#[async_trait]
impl Tool for ResolveIssueOrderTool {
    fn name(&self) -> &str {
        "resolve_issue_order"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "resolve_issue_order".to_string(),
            description: "Resolve dependency-aware execution order for a set of GitHub issues \
                using blocked-by relationships. Returns topologically sorted issue numbers."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "repo": {
                        "type": "string",
                        "description": "GitHub repository in owner/repo format (e.g. 'senara-solutions/mika')"
                    },
                    "issues": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "List of GitHub issue numbers to sort by dependency order"
                    }
                },
                "required": ["repo", "issues"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let repo = input["repo"].as_str().unwrap_or("").trim();
        if repo.is_empty() {
            return Ok(ToolOutput::error("'repo' is required (e.g. 'owner/repo')."));
        }

        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Ok(ToolOutput::error(
                "'repo' must be in owner/repo format (e.g. 'senara-solutions/mika').",
            ));
        }
        let owner = parts[0];
        let repo_name = parts[1];

        let issues: Vec<u64> = match input["issues"].as_array() {
            Some(arr) => {
                let mut nums = Vec::with_capacity(arr.len());
                for v in arr {
                    match v.as_u64() {
                        Some(n) if n > 0 => nums.push(n),
                        _ => {
                            return Ok(ToolOutput::error(format!(
                                "All items in 'issues' must be positive integers. Got: {v}"
                            )));
                        }
                    }
                }
                nums
            }
            None => {
                return Ok(ToolOutput::error(
                    "'issues' is required and must be an array of issue numbers.",
                ));
            }
        };

        // Deduplicate while preserving order (first occurrence wins).
        let mut seen = HashSet::new();
        let issues: Vec<u64> = issues.into_iter().filter(|n| seen.insert(*n)).collect();

        // Empty list: trivial result.
        if issues.is_empty() {
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "sorted": [],
                    "edges": {},
                    "external_blockers": {},
                    "cycle": null
                })
                .to_string(),
            ));
        }

        // Fail-open: no GitHub token → return input order with warning.
        let token = match ctx.github_token {
            Some(t) => t,
            None => {
                // substrate-ok: a `tracing::warn!` message — the operator's channel by
                // construction, never a tool-result `content`.
                warn!("resolve_issue_order: no GitHub token configured, returning input order");
                // mika#1964 — this is a `ToolOutput::success`, and that is exactly why
                // the guard's rule 2 is anchored on the literal rather than on
                // `ToolOutput::error(`: a rule keyed on the constructor, which is the
                // letter of the ticket's own AC, would not see this site at all. The
                // fail-open behaviour is unchanged (input order, same JSON shape); only
                // the `warning` string stops naming the credential.
                let mut out = ToolOutput::success(
                    serde_json::json!({
                        "sorted": issues,
                        "edges": {},
                        "external_blockers": {},
                        "cycle": null,
                        "warning": "Dependency resolution is not available — these issues are \
                                    returned in the order they were given, NOT in dependency \
                                    order. Do not treat this order as resolved."
                    })
                    .to_string(),
                );
                crate::tools::attach_substrate_diagnostic(
                    &mut out,
                    // substrate-diagnostic: the operator channel.
                    "resolve_issue_order has no GitHub credential on this agent, so \
                     `blockedBy` edges could not be fetched and the input order was \
                     returned unchanged.",
                    "resolve_issue_order",
                    ctx,
                )
                .await;
                return Ok(out);
            }
        };

        // Fetch blocked-by edges for each issue.
        let mut blocked_by: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut fetch_errors: Vec<String> = Vec::new();

        for &number in &issues {
            match fetch_open_blockers(token, owner, repo_name, number).await {
                Ok(blockers) => {
                    if !blockers.is_empty() {
                        blocked_by.insert(number, blockers);
                    }
                }
                Err(e) => {
                    fetch_errors.push(format!("#{number}: {e}"));
                }
            }
        }

        let result = topological_sort(&issues, &blocked_by);

        // Build edges JSON: map issue -> sorted blocker list.
        let edges_json: serde_json::Map<String, Value> = result
            .edges
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect();

        let external_json: serde_json::Map<String, Value> = result
            .external_blockers
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect();

        let mut output = serde_json::json!({
            "sorted": result.sorted,
            "edges": edges_json,
            "external_blockers": external_json,
            "cycle": result.cycle,
        });

        if !fetch_errors.is_empty() {
            output["fetch_errors"] = serde_json::json!(fetch_errors);
        }

        Ok(ToolOutput::success(output.to_string()))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_chain() {
        // 5 blocked by 4, 4 by 3, 3 by 2, 2 by 1
        let issues = vec![1, 2, 3, 4, 5];
        let mut blocked_by = HashMap::new();
        blocked_by.insert(5, vec![4]);
        blocked_by.insert(4, vec![3]);
        blocked_by.insert(3, vec![2]);
        blocked_by.insert(2, vec![1]);

        let result = topological_sort(&issues, &blocked_by);
        assert_eq!(result.sorted, vec![1, 2, 3, 4, 5]);
        assert!(result.cycle.is_none());
    }

    #[test]
    fn test_no_dependencies() {
        let issues = vec![5, 3, 1, 4, 2];
        let blocked_by = HashMap::new();

        let result = topological_sort(&issues, &blocked_by);
        // Ascending tiebreaker via min-heap
        assert_eq!(result.sorted, vec![1, 2, 3, 4, 5]);
        assert!(result.cycle.is_none());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn test_diamond() {
        // 1 blocked by 2 & 3, 2 blocked by 4, 3 blocked by 4
        let issues = vec![1, 2, 3, 4];
        let mut blocked_by = HashMap::new();
        blocked_by.insert(1, vec![2, 3]);
        blocked_by.insert(2, vec![4]);
        blocked_by.insert(3, vec![4]);

        let result = topological_sort(&issues, &blocked_by);
        assert_eq!(result.sorted, vec![4, 2, 3, 1]);
        assert!(result.cycle.is_none());
    }

    #[test]
    fn test_single_issue() {
        let issues = vec![42];
        let blocked_by = HashMap::new();

        let result = topological_sort(&issues, &blocked_by);
        assert_eq!(result.sorted, vec![42]);
        assert!(result.cycle.is_none());
    }

    #[test]
    fn test_empty_list() {
        let issues: Vec<u64> = vec![];
        let blocked_by = HashMap::new();

        let result = topological_sort(&issues, &blocked_by);
        assert!(result.sorted.is_empty());
        assert!(result.cycle.is_none());
    }

    #[test]
    fn test_external_blocker() {
        // Issue 2 blocked by [1, 999]. 999 is not in the list → external.
        let issues = vec![1, 2];
        let mut blocked_by = HashMap::new();
        blocked_by.insert(2, vec![1, 999]);

        let result = topological_sort(&issues, &blocked_by);
        assert_eq!(result.sorted, vec![1, 2]);
        assert!(result.cycle.is_none());
        assert_eq!(result.external_blockers.get(&2), Some(&vec![999]));
        assert_eq!(result.edges.get(&2), Some(&vec![1]));
    }

    #[test]
    fn test_cycle() {
        // 1 blocked by 2, 2 blocked by 1
        let issues = vec![1, 2];
        let mut blocked_by = HashMap::new();
        blocked_by.insert(1, vec![2]);
        blocked_by.insert(2, vec![1]);

        let result = topological_sort(&issues, &blocked_by);
        assert!(result.sorted.is_empty(), "cycle should clear sorted");
        assert_eq!(result.cycle, Some(vec![1, 2]));
    }

    #[test]
    fn test_partial_cycle() {
        // 3 is independent. 1 blocked by 2, 2 blocked by 1 (cycle).
        let issues = vec![1, 2, 3];
        let mut blocked_by = HashMap::new();
        blocked_by.insert(1, vec![2]);
        blocked_by.insert(2, vec![1]);

        let result = topological_sort(&issues, &blocked_by);
        // Even though 3 could be sorted, cycle presence clears sorted.
        assert!(result.sorted.is_empty());
        assert_eq!(result.cycle, Some(vec![1, 2]));
    }

    #[tokio::test]
    async fn test_no_token_fallback() {
        use crate::test_utils::test_helpers::TestHarness;

        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // TestHarness ctx has github_token = None

        let tool = ResolveIssueOrderTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "repo": "owner/repo",
                    "issues": [3, 1, 2]
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error, "got error: {}", result.content);
        // mika#1964 — on operator tier the routed diagnostic is folded back into
        // `content` after a blank line, so the JSON is the first block rather than
        // the whole string. The JSON itself is unchanged and complete; only an
        // operator-facing paragraph follows it. That is a declared change of served
        // contract for the two JSON-bodied tools of the sweep (this one and
        // `pr_merge_with_gate`), and it is what buys the family tier a `content`
        // that names no credential at all.
        let json_block = result.content.split("\n\n").next().unwrap();
        let json: Value = serde_json::from_str(json_block).unwrap();
        // Should return issues in input order
        assert_eq!(json["sorted"], serde_json::json!([3, 1, 2]));
        assert!(json["warning"].as_str().is_some());
        // The warning must still say the order is NOT resolved — that is the fact
        // the turn needs. What it no longer says is which credential is missing.
        let warning = json["warning"].as_str().unwrap();
        assert!(warning.contains("NOT in dependency order"), "{warning}");
        assert!(!warning.contains("GitHub token"), "{warning}");
        // Operator tier: the credential cause is folded back in after the JSON.
        assert!(
            result.content.contains("no GitHub credential"),
            "operator tier lost the actionable detail: {}",
            result.content
        );
    }

    /// mika#1964 V3 — family tier reads no credential, and the operator detail is
    /// routed instead of dropped.
    ///
    /// This site is a `ToolOutput::success`, which is why the guard's rule 2 is
    /// anchored on the literal and not on `ToolOutput::error(` as the ticket's
    /// acceptance criterion proposes: a constructor-keyed rule would not see it.
    #[tokio::test]
    async fn mika1964_no_token_fallback_does_not_leak_on_family_tier() {
        use crate::test_utils::test_helpers::TestHarness;

        let harness = TestHarness::new();
        let mut ctx = harness.ctx();
        ctx.tier = mika_common::home::AgentTier::Family;

        let result = ResolveIssueOrderTool
            .execute(
                serde_json::json!({"repo": "owner/repo", "issues": [3, 1, 2]}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        for token in ["GitHub token", "MIKA_GITHUB_TOKEN", "credential"] {
            assert!(
                !result.content.contains(token),
                "family tier leaked {token:?}: {}",
                result.content
            );
        }
        // Family tier gets the JSON and nothing appended — so it stays parseable
        // whole, and the fail-open behaviour is untouched.
        let json: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["sorted"], serde_json::json!([3, 1, 2]));

        let events = harness
            .db
            .get_audit_events("test-session")
            .await
            .expect("get_audit_events");
        assert!(
            events.iter().any(|e| {
                e.tool_name == "substrate_unavailable" && e.target_key == "resolve_issue_order"
            }),
            "the operator detail was dropped instead of routed: {events:?}"
        );
    }

    #[tokio::test]
    async fn test_empty_issues_input() {
        use crate::test_utils::test_helpers::TestHarness;

        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = ResolveIssueOrderTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "repo": "owner/repo",
                    "issues": []
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let json: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["sorted"], serde_json::json!([]));
        assert!(json["cycle"].is_null());
    }

    #[tokio::test]
    async fn test_invalid_repo_format() {
        use crate::test_utils::test_helpers::TestHarness;

        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = ResolveIssueOrderTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "repo": "invalid",
                    "issues": [1]
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("owner/repo"));
    }

    #[tokio::test]
    async fn test_missing_repo() {
        use crate::test_utils::test_helpers::TestHarness;

        let harness = TestHarness::new();
        let ctx = harness.ctx();

        let tool = ResolveIssueOrderTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "issues": [1]
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("'repo' is required"));
    }
}
