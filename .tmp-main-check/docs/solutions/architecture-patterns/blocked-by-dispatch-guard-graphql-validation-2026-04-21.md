---
title: "Blocked-By Dispatch Guard: GitHub GraphQL Validation Before Long-Running Dispatch"
date: 2026-04-21
category: architecture-patterns
module: mika-agent (skills/executor.rs)
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - "Adding external API calls to the dispatch validation chain"
  - "Querying GitHub's GraphQL API from engine-level Rust code"
  - "Extending validate_dispatch_readiness() with new checks"
tags:
  - dispatch-guard
  - blocked-by
  - graphql
  - github-api
  - injection-safety
  - fail-open
  - fail-closed
  - executor
  - long-running
related_issues:
  - "#713"
  - "#525"
  - "#583"
---

# Blocked-By Dispatch Guard: GitHub GraphQL Validation Before Long-Running Dispatch

## Context

The `validate_dispatch_readiness()` function in `skills/executor.rs` enforces pre-dispatch checks before spawning long-running exec handlers. Checks #1–#4 are all DB-only (task status, active callback child, global dispatch guard, per-turn dispatch limit). Issue #713 added check #5: querying GitHub's GraphQL API for `blockedByIssues` edges to reject dispatch when upstream blockers are still open.

This was the first GraphQL call in the codebase (all prior GitHub API usage is REST via `github_get()`). The key design decisions and pitfalls are documented here.

## Guidance

### GraphQL Variable Injection Safety

**Always use GraphQL variables** (`$owner: String!`) with a `variables` object for user-derived values. Never interpolate strings into a GraphQL query via `format!()`.

```rust
// WRONG — vulnerable to injection via owner/repo containing quotes
let query = format!(
    r#"{{"query":"query {{ repository(owner: \"{owner}\", name: \"{repo}\") ... }}"}}"#
);

// RIGHT — serde_json handles escaping; GraphQL engine handles variable binding
let query_str = "query($owner:String!,$repo:String!,$number:Int!) { ... }";
let body = serde_json::json!({
    "query": query_str,
    "variables": { "owner": owner, "repo": repo, "number": number }
});
client.post(url).json(&body)
```

`parse_github_ref()` constrains owner/repo to URL path segments but does not reject `"`, `\`, `{`, `}` characters. A stored `reference_url` with a crafted owner like `o"}` would break JSON string boundaries in the `format!()` approach. The `serde_json::json!` + variables approach eliminates this class of injection entirely.

### Check Ordering: Expensive Checks Last

The blocked-by check makes an external HTTP call (10s timeout). It runs after all four cheap DB checks so that tasks failing those checks never incur the network round-trip. This is consistent with the validation ordering principle documented in `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`.

### Fail-Open vs Fail-Closed Decision Matrix

| Condition | Behavior | Rationale |
|-----------|----------|-----------|
| No `github_token` configured | **Fail-open** (skip with `warn!`) | Configuration state — the operator chose not to configure GitHub integration |
| No `reference_url` on task | **Skip** (no check needed) | Nothing to query |
| `reference_url` is a PR (not issue) | **Skip** | `blockedByIssues` is issue-only; PRs don't have this connection |
| GitHub API returns error (401, 403, 429, 5xx, network) | **Fail-closed** (reject dispatch) | Infrastructure failure — cannot verify blocker state |
| GraphQL-level error in response body | **Fail-closed** (reject dispatch) | Same rationale as HTTP errors |
| `blockedByIssues` field missing from response | **Fail-open** (treat as no blockers) | Repo may not support sub-issues feature |

### Shared Parsing Function Pattern

When the HTTP fetch and response parsing are in the same function, extract the parsing into a standalone pure function for testability:

```rust
// Production: fetch_open_blockers() calls this after the HTTP round-trip
fn extract_open_blocker_numbers(body: &serde_json::Value) -> Vec<u64> { ... }

// Tests: call extract_open_blocker_numbers() directly with fixture JSON
#[test]
fn test_parse_blockers_some_open() {
    let body = serde_json::json!({ "data": { ... } });
    assert_eq!(extract_open_blocker_numbers(&body), vec![689, 691]);
}
```

Do NOT duplicate parsing logic in a test helper — the test must exercise the production code path.

### Reusing `parse_github_ref` Across Modules

`parse_github_ref()` and `GitHubRef` in `tools/check_task.rs` are `pub(crate)` and re-exported from `tools/mod.rs`:

```rust
// tools/mod.rs
mod check_task;
pub(crate) use check_task::{GitHubRef, parse_github_ref};
```

This exposes only the needed items without making the entire `check_task` module public. The module stays private; the re-export is the controlled surface.

## Why This Matters

- **Injection safety**: GraphQL string interpolation is the same class of vulnerability as SQL injection. Using variables is the parameterized-query equivalent.
- **Observability**: The `warn!` on both the fail-open (no token) and fail-closed (API error) paths ensures dispatch rejections are visible in structured logs without requiring tool_calls table grepping.
- **Test fidelity**: Duplicated parsing logic in test helpers creates false confidence — the production code can regress while tests pass against a stale copy.

## When to Apply

- Adding any new GitHub GraphQL call to the codebase
- Extending `validate_dispatch_readiness()` with additional checks
- Building engine-level validation that depends on external API state
- Reusing private functions across module boundaries

## Examples

The complete implementation is in `crates/mika-agent/src/skills/executor.rs`:
- `fetch_open_blockers()` — GraphQL query with variables
- `extract_open_blocker_numbers()` — pure parsing function
- `validate_dispatch_readiness()` — check #5 integration with fail-open/fail-closed branching

## Related

- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — original guard (#525)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — when to use engine enforcement vs prompt rules
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — validation ordering principle
- `docs/solutions/code-review-patterns/extract-shared-github-get-helper.md` — REST API helper pattern
