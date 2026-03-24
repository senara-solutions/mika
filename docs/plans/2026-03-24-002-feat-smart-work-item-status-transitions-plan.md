---
title: "feat: Add smart work item status transitions"
type: feat
status: completed
date: 2026-03-24
---

# feat: Add smart work item status transitions

## Overview

Add validated status transitions to `update_work_item_status` and a new `check_work_item` tool that reads work item details with optional GitHub PR/issue status enrichment. Together these enable two interaction patterns: **direct status change** (agent validates and updates immediately) and **inspect-then-confirm** (agent reads work item, fetches PR status if available, presents findings, waits for user decision).

## Problem Statement / Motivation

Currently `update_work_item_status` allows **free transitions** — any status can move to any other status. This means:
- The agent can transition `completed → pending` without warning, silently re-opening closed work
- There is no guard against nonsensical transitions (e.g., `cancelled → blocked`)
- The system prompt _suggests_ `pending → in_progress → blocked → completed` but nothing enforces it

Additionally, when work items have a `reference_url` pointing to a GitHub PR, the agent has no way to check whether the PR is open, merged, or closed. It must rely on the user's assertion, which is unreliable after conversation compaction.

## Proposed Solution

Three changes, ordered by dependency:

1. **Transition validation in `update_work_item_status`** — enforce a defined state machine; reject invalid transitions with clear guidance on permitted next states
2. **New `check_work_item` tool** — read work item details from DB, optionally fetch GitHub PR/issue status via the GitHub REST API, return a structured summary
3. **System prompt refinement** — teach the agent when to use each interaction pattern (direct update vs. inspect-then-confirm)

## Technical Approach

### Architecture

#### Status Transition State Machine

Define an explicit transition matrix. The design allows all forward transitions plus `blocked → pending` (un-block regression). Terminal states (`completed`, `cancelled`) are final — no re-opening.

```
                    ┌──────────────────────────────────┐
                    │                                  ▼
  pending ──► in_progress ──► blocked ──► completed
    │              │              │
    │              │              ├──► cancelled
    │              │              │
    │              │              └──► in_progress (un-block)
    │              │
    │              ├──► completed
    │              └──► cancelled
    │
    ├──► blocked
    ├──► completed
    └──► cancelled
```

**Explicit transition table:**

| From ╲ To | pending | in_progress | blocked | completed | cancelled |
|-----------|---------|-------------|---------|-----------|-----------|
| pending | — | ✅ | ✅ | ✅ | ✅ |
| in_progress | ✗ | — | ✅ | ✅ | ✅ |
| blocked | ✗ | ✅ | — | ✅ | ✅ |
| completed | ✗ | ✗ | ✗ | — | ✗ |
| cancelled | ✗ | ✗ | ✗ | ✗ | — |

Rationale:
- `pending` can go anywhere — work hasn't started yet, any path is valid
- `in_progress → pending` blocked — don't regress active work to not-started
- `blocked → in_progress` allowed — the un-block case
- `blocked → pending` blocked — if you're unblocked, resume in_progress, don't regress
- Terminal states are final — `completed`/`cancelled` items stay resolved. If the user truly needs to re-open, they create a new work item. This matches `validate_work_item()` which already treats these as non-active.

Implementation: a `const VALID_TRANSITIONS` in `update_work_item_status.rs`:
```rust
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    ("pending", &["in_progress", "blocked", "completed", "cancelled"]),
    ("in_progress", &["blocked", "completed", "cancelled"]),
    ("blocked", &["in_progress", "completed", "cancelled"]),
    ("completed", &[]),
    ("cancelled", &[]),
];
```

The tool fetches the current status first (already does), then checks if the target is in the allowed list. On rejection, returns a clear message: `"Cannot transition from '{current}' to '{target}'. Valid transitions from '{current}': {list}. Completed and cancelled items are final."`.

**DB-level change:** `update_manual_task_status` already returns the old status. The validation happens in the tool layer (not DB), keeping the DB method general-purpose. The `completed_at` CASE logic is unchanged.

#### check_work_item Tool

A new tool `check_work_item` in `crates/mika-agent/src/tools/check_work_item.rs`.

**Architecture decision:** Follow the `brave_api_key` pattern — add `github_token: Option<&'a str>` to `ToolContext`. The tool is a **unit struct** registered in `default_tools()`. When `github_token` is `None`, the tool still works but skips GitHub API enrichment. This is the simplest approach — no stateful struct needed, no separate registration path.

The `reqwest::Client` is NOT added to ToolContext. Instead, the tool constructs a one-shot client for the API call. This is acceptable because:
- `check_work_item` is user-initiated (not high-frequency)
- Connection pooling benefit is negligible for single calls
- Avoids threading a client through every ToolContext construction site

**Input:** `{ "task_id": "<uuid>" }`

**Output (structured text):**
```
Work item: <id>
Label: <label>
Status: <status>
Source: <source>
Reference: <url>
Created: <timestamp>
Updated: <timestamp>
Completed: <timestamp or n/a>

GitHub PR Status:
  State: open | closed (merged) | closed (not merged) | draft
  Branch: <head_ref>
  Checks: passing | failing | pending | n/a

Children (3): 2 completed, 1 in_progress
```

**GitHub URL parsing:** A helper function `parse_github_ref(url: &str) -> Option<GitHubRef>`:
```rust
enum GitHubRef {
    PullRequest { owner: String, repo: String, number: u64 },
    Issue { owner: String, repo: String, number: u64 },
}
```

Parses `https://github.com/{owner}/{repo}/pull/{number}` and `/issues/{number}`. Only `github.com` is supported (no enterprise domains). This is a URL parser, **not** a fetcher — the actual API call goes to `https://api.github.com/repos/{owner}/{repo}/pulls/{number}` (never the raw URL). This prevents SSRF.

**GitHub API call pattern** (following `CreateGithubIssueTool`):
- `GET https://api.github.com/repos/{owner}/{repo}/pulls/{number}`
- Headers: `Authorization: Bearer {token}`, `User-Agent: mika-agent`, `Accept: application/vnd.github+json`
- Extract: `state`, `merged`, `draft`, `head.ref`
- Error mapping: 401 → "token invalid", 403 → "lacks permission", 404 → "PR not found", 429 → "rate limited"
- Timeout: 10 seconds
- On any failure: return work item data without GitHub enrichment, append a note explaining why

**Graceful degradation priority:**
1. No `github_token` → skip API call, report `"GitHub status: not available (no token configured)"`
2. `reference_url` not a GitHub URL → skip API call, report `"Reference: <url> (not a GitHub PR/issue)"`
3. API error → report work item data + `"GitHub status: unavailable ({error})"`

#### System Prompt Updates

Update the work item guidance in `prompt.rs` (conversation mode, around line 398):

**Current:**
```
Use update_work_item_status to progress work items through their lifecycle
(pending → in_progress → blocked → completed).
```

**New:**
```
Work item status management follows two patterns:
- **Direct update:** When the user explicitly requests a status change ("mark it done",
  "cancel the task"), call update_work_item_status directly. The tool validates transitions
  — terminal states (completed, cancelled) are final.
- **Inspect first:** When the user asks about a work item's state ("check the task",
  "is the PR merged?"), call check_work_item to read details and any linked GitHub PR status.
  Present findings and wait for the user's decision before changing status.
Status transitions: pending → in_progress/blocked/completed/cancelled,
in_progress → blocked/completed/cancelled, blocked → in_progress/completed/cancelled.
Completed and cancelled are terminal — cannot be re-opened.
```

### Implementation Phases

#### Phase 1: Transition Validation

**Files:**
- `crates/mika-agent/src/tools/update_work_item_status.rs` — add `VALID_TRANSITIONS` const, validation logic before DB call, clear error messages with valid alternatives
- Tests: add tests for valid transitions, rejected transitions, terminal-state rejection

**Tasks:**
- [x] Define `VALID_TRANSITIONS` constant
- [x]Add `fn is_valid_transition(from: &str, to: &str) -> bool` helper
- [x]Add `fn allowed_transitions(from: &str) -> &[&str]` helper
- [x]Insert validation between status parse and DB call in `execute()`
- [x]Update tool description to reflect validated (not free) transitions
- [x]Add tests: `test_valid_forward_transitions`, `test_rejected_backward_transition`, `test_terminal_state_cannot_transition`, `test_blocked_to_in_progress_allowed`

**Estimated effort:** Small — pure logic addition to existing tool, ~50 lines of code + ~80 lines of tests.

#### Phase 2: check_work_item Tool

**Files:**
- `crates/mika-agent/src/tools/check_work_item.rs` — new file, the tool implementation
- `crates/mika-agent/src/tools/mod.rs` — add `pub mod check_work_item;`, register in `default_tools()`, add `github_token: Option<&'a str>` to `ToolContext`
- `crates/mika-agent/src/db.rs` — add `get_manual_task(task_id, agent_id) -> Result<Option<Task>>` (agent-scoped, manual-only variant of `get_task_unscoped`)
- All `ToolContext` construction sites — pass through `github_token` from `Settings`

**Tasks:**
- [x]Add `github_token: Option<&'a str>` to `ToolContext` struct
- [x]Thread `github_token` through all ToolContext construction sites (agent loop, server handler, team engine, test harness)
- [x]Add `get_manual_task(task_id, agent_id)` to `Database` and `AsyncDatabase`
- [x]Create `parse_github_ref(url: &str) -> Option<GitHubRef>` helper
- [x]Implement `CheckWorkItemTool` (DB read + optional GitHub API call)
- [x]Register in `default_tools()`
- [x]Add tests: `test_check_basic`, `test_check_with_github_pr`, `test_check_no_github_token`, `test_check_non_github_url`, `test_check_not_found`, `test_parse_github_pr_url`, `test_parse_github_issue_url`, `test_parse_non_github_url`

**Estimated effort:** Medium — new tool file (~200 lines), ToolContext change (~10 call sites), URL parser (~30 lines), tests (~200 lines).

#### Phase 3: System Prompt Updates

**Files:**
- `crates/mika-agent/src/prompt.rs` — update conversation mode work item guidance

**Tasks:**
- [x]Replace work item guidance paragraph (around line 398)
- [x]Add `check_work_item` to the tool list if not already auto-discovered
- [x]Verify prompt test snapshots still pass (update expected strings)

**Estimated effort:** Small — ~10 lines of prompt text change.

## System-Wide Impact

### Interaction Graph

`update_work_item_status` call chain (unchanged):
1. Agent calls `update_work_item_status` → tool validates transition → `db.update_manual_task_status()` → audit log
2. `validate_work_item()` in `tools/mod.rs` checks active statuses — **no change needed** (it already rejects completed/cancelled)
3. Heartbeat prompt injects active work items — **no change needed** (only shows pending/in_progress/blocked)
4. `completed_at` CASE logic in DB — **no change needed** (still sets on completed, clears otherwise)

`check_work_item` call chain (new):
1. Agent calls `check_work_item` → tool reads from DB → optionally calls GitHub API → returns structured text
2. No state mutation — purely read-only tool
3. No callbacks, no event emission, no side effects

### Error Propagation

- Transition validation errors are returned as `ToolOutput::error()` — the agent sees them and relays to user
- GitHub API errors are caught and degraded — never surface as tool errors, just missing enrichment
- `reqwest` timeout (10s) prevents blocking the agent loop

### State Lifecycle Risks

- **No new state** is introduced. The transition validation constrains an existing field.
- **No partial failure risk** — `update_work_item_status` is a single SQL UPDATE (atomic).
- **`check_work_item` is read-only** — no mutation, no partial state.

### API Surface Parity

- `update_work_item_status` — behavior change (tighter validation). Breaking for any automation that relied on free transitions (e.g., `completed → pending`). Pre-1.0, this is acceptable.
- `check_work_item` — new tool, additive.
- System prompt — internal, not an API surface.

### Integration Test Scenarios

1. **User creates work item with PR reference, checks it, then completes it:** `create_work_item` → `check_work_item` (GitHub returns merged PR) → `update_work_item_status` to completed → verify all tools return success
2. **User tries to re-open a completed work item:** `update_work_item_status` from completed → in_progress → verify rejection with clear message
3. **check_work_item with no GitHub token configured:** verify work item data is returned without GitHub section, no error
4. **check_work_item with broken GitHub token:** verify degraded output (work item data + error note)

## Acceptance Criteria

- [x]`update_work_item_status` rejects invalid transitions with a message listing valid alternatives
- [x]All forward transitions work (pending → any, in_progress → blocked/completed/cancelled, blocked → in_progress/completed/cancelled)
- [x]Terminal states (completed, cancelled) cannot transition to any other state
- [x]New `check_work_item` tool returns work item details including status, label, reference_url, timestamps
- [x]When `reference_url` is a GitHub PR URL and token is configured, tool fetches and includes PR state (open/closed/merged/draft)
- [x]When `reference_url` is a GitHub issue URL, tool fetches and includes issue state (open/closed)
- [x]When no GitHub token is configured, tool returns work item data with a "not available" note (no error)
- [x]GitHub API failures degrade gracefully (work item data still returned)
- [x]`ToolContext` carries `github_token: Option<&str>`, threaded through all construction sites
- [x]System prompt updated with two-pattern guidance (direct update vs. inspect-then-confirm)
- [x]All existing tests pass unchanged
- [x]New tests cover transition validation (valid, rejected, terminal), check_work_item (basic, GitHub PR, no token, not found), and URL parsing

## Success Metrics

- Zero regressions in existing ~1596 tests
- Transition validation catches 100% of invalid state machine transitions
- `check_work_item` returns useful output with and without GitHub API access

## Dependencies & Risks

**Dependencies:**
- `reqwest` — already a dependency (used by `ClaudeClient`, investigation tools)
- `MIKA_INVESTIGATE_GITHUB_TOKEN` — reuses existing config key (no new env var)

**Risks:**
- **ToolContext change** touches ~10 call sites. Low risk — adding an `Option` field with default `None` is backward-compatible. Compiler enforces all sites are updated.
- **Breaking change for free transitions** — any external automation relying on `completed → pending` will break. Pre-1.0, acceptable per versioning policy. Document in PR description.
- **GitHub API rate limiting** — unauthenticated: 60 req/hour, authenticated: 5000 req/hour. Since `check_work_item` is user-initiated, rate limits are unlikely to be hit.

## Sources & References

### Internal References

- `crates/mika-agent/src/tools/update_work_item_status.rs` — current tool, free transitions
- `crates/mika-agent/src/tools/mod.rs:64-83` — `ToolContext` struct, `brave_api_key` pattern
- `crates/mika-agent/src/tools/mod.rs:210-240` — `validate_work_item()` helper
- `crates/mika-agent/src/tools/mod.rs:467-469` — work item tool registration in `default_tools()`
- `crates/mika-agent/src/server/investigate.rs:434-559` — `CreateGithubIssueTool` GitHub API pattern
- `crates/mika-agent/src/db.rs:2480-2514` — `update_manual_task_status` DB method
- `crates/mika-agent/src/prompt.rs:397-406` — system prompt work item guidance
- `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — prevention checklist for agent-facing tools
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — code guard pattern
- `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — audit ALL status queries when changing state machine

### Related Work

- Issue: #257
