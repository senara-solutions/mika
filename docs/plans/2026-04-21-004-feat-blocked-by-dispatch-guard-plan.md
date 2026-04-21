---
title: "feat: Pre-dispatch blocked-by guard in validate_dispatch_readiness"
type: feat
status: active
date: 2026-04-21
---

# feat: Pre-dispatch blocked-by guard in validate_dispatch_readiness

## Overview

Add a fifth check to `validate_dispatch_readiness()` that queries GitHub's GraphQL API for `blockedByIssues` edges on the ticket being dispatched. If any blocker is still open, dispatch is rejected with a structured error. This makes GitHub blocked-by relationships load-bearing rather than decorative.

## Problem Frame

The self-dev workflow sets up `blocked by` relationships on GitHub issues, but these are purely informational — the engine ignores them. When the agent dispatches work on an issue whose blockers are still open, the resulting session wastes compute and may produce conflicting changes. An engine-level guard (consistent with the existing four-check pattern in `validate_dispatch_readiness()`) structurally prevents this class of wasted dispatch.

## Requirements Trace

- R1. Query GitHub `blockedByIssues` edges for the dispatched ticket and reject if any blocker has `state != CLOSED`
- R2. Return structured JSON error `dispatch_blocked_by` with blocking issue numbers for LLM feedback
- R3. Fail-open when no GitHub token is configured (skip check with warning)
- R4. Skip check when the task has no GitHub issue reference
- R5. Place the check last in the validation chain (most expensive — external API call)
- R6. Guard must run before any state mutations (auto-transition, callback creation)

## Scope Boundaries

- Only `blockedByIssues` edges are checked — not PR checks, not milestone dependencies
- No REST fallback — `blockedByIssues` is GraphQL-only on GitHub's API
- No caching of blocker state — each dispatch re-queries (dispatches are infrequent)
- No new database schema changes

### Deferred to Separate Tasks

- Extracting `github_get` / `parse_github_ref` into a shared `tools::github` module: separate refactor PR
- Adding a `github_graphql` helper to `mika-common`: only warranted when a second GraphQL callsite exists

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/executor.rs` — `validate_dispatch_readiness()` (line 562): four existing checks (task fetch, status gate, active callback child, global dispatch guard). Returns `Result<String, String>` with JSON error strings.
- `crates/mika-agent/src/tools/check_task.rs` — `parse_github_ref()` (line 34): parses `reference_url` into `GitHubRef { owner, repo, number }`. Currently private. `github_get()` (line 85): authenticated REST GET helper with timeout, headers, error mapping.
- `crates/mika-agent/src/tools/mod.rs` — `ToolContext.github_token: Option<&'a str>` (line 100)
- `crates/mika-agent/src/skills/executor.rs` — `execute_long_running()` (line 698): already receives `github_token: Option<&str>`, calls `validate_dispatch_readiness(db, task_id)` at line 718
- `Task.reference_url: Option<String>` — available on the task struct already fetched in check #1

### Institutional Learnings

- **Dispatch-readiness guard doc** (`docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`): fail-closed on DB errors is the established pattern. However, the issue specifies fail-open for missing token, and this is consistent — DB errors are internal failures (should block), while missing token is a configuration choice (should degrade gracefully).
- **Engine guards vs prompt rules** (`docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`): blocked-by is "against-gradient" behavior — the LLM's default is to dispatch when instructed. Engine enforcement is the correct layer.
- **Validation ordering** (`docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`): cheap checks before expensive ones. The blocked-by check (external API call) should be last.
- **SSRF prevention** (`docs/solutions/architecture-patterns/work-item-status-transition-validation.md`): never follow raw URLs. Parse to extract owner/repo/number, then construct API URL programmatically.
- **Phantom retry guard** (`docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md`): guard must run before state mutations.

### External References

- GitHub GraphQL API: `blockedByIssues` connection on `Issue` type (requires `repository` scope or read access)
- GitHub GraphQL endpoint: `POST https://api.github.com/graphql` with `Authorization: Bearer {token}`

## Key Technical Decisions

- **GraphQL, not REST:** `blockedByIssues` is only available via GitHub's GraphQL API. No REST equivalent exists. This will be the first GraphQL call in the codebase.
- **Inline helper, not shared module:** The GraphQL POST helper will live in `executor.rs` as a private async function (similar to how `github_get` is private in `check_task.rs`). Extracting to a shared module is deferred until a second callsite exists.
- **Reuse `parse_github_ref` by making it `pub(crate)`:** The function is well-tested and handles URL parsing correctly. Making it public avoids duplication and SSRF risk.
- **Fail-open on missing token, fail-closed on API errors:** Missing token is a configuration state (skip with `warn!`). API failures (network, auth, rate limit) reject dispatch to prevent dispatching into unknown blocker state — consistent with the existing fail-closed pattern for infrastructure errors, but distinct from the "no config" case.
- **Check placement:** After the global dispatch guard (check #4), before the `Ok(task.status)` return. This is the most expensive check and should short-circuit after all cheap DB checks pass.

## Open Questions

### Resolved During Planning

- **Should `parse_github_ref` be duplicated or shared?** Shared — make it `pub(crate)` and import from `tools::check_task`. The function is stable, tested, and the SSRF-safe parsing is security-critical to get right.
- **What if `blockedByIssues` returns a PR, not an issue?** GitHub's `blockedByIssues` connection only returns issues (the `blockedByPullRequests` is a separate connection). We only need to check issues per the issue spec.
- **What about pagination?** The issue spec uses `first: 10`. In practice, issues rarely have more than 10 blockers. If they do, the first 10 are checked — any open blocker in the first page is sufficient to reject.

### Deferred to Implementation

- Exact GraphQL query string formatting — may need adjustment based on GitHub API response shape
- Whether `GitHubRef` type needs to be moved or re-exported (depends on import ergonomics)

## Implementation Units

- [x] **Unit 1: Make `parse_github_ref` and `GitHubRef` pub(crate)**

  **Goal:** Enable reuse of the GitHub URL parser from `executor.rs` without duplicating code.

  **Requirements:** R1 (prerequisite — need to extract owner/repo/number from reference_url)

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/tools/check_task.rs`
  - Modify: `crates/mika-agent/src/tools/mod.rs` (re-export if needed)

  **Approach:**
  - Change `fn parse_github_ref` to `pub(crate) fn parse_github_ref`
  - Change `enum GitHubRef` to `pub(crate) enum GitHubRef`
  - Verify existing tests still pass (visibility change only)

  **Patterns to follow:**
  - Other `pub(crate)` helpers in the tools module

  **Test scenarios:**
  - Happy path: existing `parse_github_ref` tests continue to pass unchanged
  - Edge case: no new test needed — this is a visibility-only change

  **Verification:**
  - `cargo test -p mika-agent` passes
  - `parse_github_ref` is importable from `crate::tools::check_task` in `executor.rs`

- [x] **Unit 2: Add GitHub GraphQL blocked-by query function**

  **Goal:** Create an async function that queries GitHub's GraphQL API for `blockedByIssues` edges and returns a list of open blockers.

  **Requirements:** R1, R3, R4

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/skills/executor.rs`
  - Test: `crates/mika-agent/src/skills/executor.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - Add `async fn fetch_open_blockers(token: &str, owner: &str, repo: &str, number: u64) -> Result<Vec<u64>, String>` as a private function in `executor.rs`
  - POST to `https://api.github.com/graphql` with the `blockedByIssues` query
  - Parse the response to extract issue numbers where `state != "CLOSED"`
  - Return `Ok(vec![])` when no open blockers, `Ok(vec![numbers...])` when blockers exist, `Err(reason)` on API failure
  - Use `reqwest::Client` with 10s timeout, same headers as `github_get` (Bearer auth, User-Agent, Accept)

  **Patterns to follow:**
  - `github_get()` in `check_task.rs` for HTTP client construction, header setup, error mapping
  - `serde_json::Value` for response parsing (no need for full deserialization structs)

  **Test scenarios:**
  - Happy path: response with all-closed blockers returns empty vec
  - Happy path: response with no `blockedByIssues` nodes returns empty vec
  - Edge case: response with mix of open and closed blockers returns only open issue numbers
  - Error path: non-200 status returns descriptive Err string
  - Error path: malformed JSON response returns parse error
  - Edge case: empty `nodes` array returns empty vec

  **Verification:**
  - Unit tests pass for response parsing logic (test the parsing, mock the HTTP call)
  - Function compiles and is callable from `validate_dispatch_readiness`

- [x] **Unit 3: Integrate blocked-by check into validate_dispatch_readiness**

  **Goal:** Wire the blocked-by query into the existing guard chain as check #5, with proper fail-open/fail-closed behavior.

  **Requirements:** R1, R2, R3, R4, R5, R6

  **Dependencies:** Units 1, 2

  **Files:**
  - Modify: `crates/mika-agent/src/skills/executor.rs` (`validate_dispatch_readiness` signature and body, `execute_long_running` call site)
  - Test: `crates/mika-agent/src/skills/executor.rs` (inline tests)

  **Approach:**
  - Add `github_token: Option<&str>` parameter to `validate_dispatch_readiness()`
  - Update the call site in `execute_long_running()` (line ~718) to pass `github_token`
  - After the global dispatch guard (check #4) and before `Ok(task.status)`:
    1. Extract `reference_url` from the task (already fetched in check #1)
    2. If `reference_url` is `None` or doesn't parse as a GitHub issue → skip (R4)
    3. If `github_token` is `None` → `warn!` and skip (R3, fail-open)
    4. Call `fetch_open_blockers(token, owner, repo, number)`
    5. If returns `Ok(blockers)` with non-empty vec → reject with structured error (R2)
    6. If returns `Err(reason)` → reject with `dispatch_check_failed` error (fail-closed on API errors)
    7. If returns `Ok(vec![])` → proceed
  - Error shape matches the issue spec: `{"error": "dispatch_blocked_by", "task_id", "blocking_issues": [...], "message": "..."}`

  **Patterns to follow:**
  - Existing check #3 (active callback child) for the match/Ok/Err structure
  - Existing check #4 (global dispatch guard) for the fail-closed DB error pattern
  - `extract_pr_url()` usage in existing checks for including `pr_url` in errors

  **Test scenarios:**
  - Happy path: task with no `reference_url` → check skipped, dispatch proceeds
  - Happy path: task with `reference_url` pointing to a PR (not issue) → check skipped
  - Happy path: task with issue reference, all blockers closed → dispatch proceeds
  - Happy path: task with issue reference, no blockers at all → dispatch proceeds
  - Error path: task with issue reference, one open blocker → rejected with `dispatch_blocked_by` error containing the blocker number
  - Error path: task with issue reference, multiple open blockers → rejected with all blocker numbers in array
  - Error path: `github_token` is `None` → check skipped with warning (fail-open)
  - Error path: GitHub API returns error → rejected with `dispatch_check_failed`
  - Integration: verify the check runs after checks #1-#4 (ordering)
  - Integration: verify `execute_long_running` passes `github_token` to the updated function

  **Verification:**
  - All existing `validate_dispatch_readiness` tests still pass (no regression)
  - New tests cover the five scenarios from the issue spec
  - `cargo clippy -p mika-agent` passes
  - `cargo test -p mika-agent` passes

## System-Wide Impact

- **Interaction graph:** `validate_dispatch_readiness()` is called only from `execute_long_running()` in `executor.rs`. No other callers. The signature change is internal (private function).
- **Error propagation:** New error codes (`dispatch_blocked_by`) flow through the existing `ToolOutput::error()` path to the LLM. Self-dev skill prompts may need awareness of this error to handle it gracefully (but that's a skill-layer concern, not engine).
- **State lifecycle risks:** None — the check runs before any state mutations (auto-transition, callback creation). Rejection leaves the task unchanged.
- **API surface parity:** No external API changes. Dashboard and A2A are unaffected.
- **Integration coverage:** The blocked-by check introduces an external dependency (GitHub API) into a previously DB-only validation path. Network failures in the check are fail-closed, consistent with existing infrastructure error handling.
- **Unchanged invariants:** The existing four checks are unmodified. The function signature gains one parameter but behavior for `github_token: None` is identical to the current (no-check) behavior.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| GitHub API rate limiting on GraphQL endpoint | 10s timeout + fail-closed means dispatch is rejected (not silently passed). Dispatches are infrequent (minutes apart), so rate limits are unlikely in practice. |
| `blockedByIssues` requires specific GitHub permissions | Same token used for `pr_merge_with_gate` and `check_task` — if those work, this will too. `repository` read scope is sufficient. |
| First GraphQL call in codebase — no established pattern | Modeled closely on `github_get()` (same client construction, headers, error mapping). Kept as a private function to avoid premature abstraction. |

## Sources & References

- Related issue: #713
- Related code: `crates/mika-agent/src/skills/executor.rs` (validate_dispatch_readiness, execute_long_running)
- Related code: `crates/mika-agent/src/tools/check_task.rs` (parse_github_ref, github_get, GitHubRef)
- Solution doc: `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`
- Solution doc: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
