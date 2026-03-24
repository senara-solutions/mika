---
title: "Work item status transition validation and check_work_item tool"
category: architecture-patterns
date: 2026-03-24
tags: [work-items, state-machine, tools, github-api, transition-validation]
modules: [mika-agent/tools/update_work_item_status, mika-agent/tools/check_work_item, mika-agent/tools/mod]
related_issues: [257]
---

# Work Item Status Transition Validation

## Problem

`update_work_item_status` allowed **free transitions** — any status could move to any other status. This meant the agent could silently re-open completed work (`completed → pending`) or make nonsensical transitions (`cancelled → blocked`). The system prompt suggested `pending → in_progress → blocked → completed` but nothing enforced it.

Additionally, when work items had a `reference_url` pointing to a GitHub PR, the agent had no way to verify the PR state before changing status. After conversation compaction, the agent couldn't remember whether a PR was actually merged.

## Root Cause

Design gap: the initial work item implementation (issue #211) intentionally used free transitions for simplicity, deferring validation. As usage grew, the lack of enforcement became a reliability issue — the agent would occasionally regress work items based on stale context.

## Solution

### 1. Transition State Machine (tool layer, not DB)

Added `VALID_TRANSITIONS` constant in `update_work_item_status.rs`:

```rust
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    ("pending", &["in_progress", "blocked", "completed", "cancelled"]),
    ("in_progress", &["blocked", "completed", "cancelled"]),
    ("blocked", &["in_progress", "completed", "cancelled"]),
    ("completed", &[]),
    ("cancelled", &[]),
];
```

Key decisions:
- **Validation in tool layer, not DB.** The DB method `update_manual_task_status` remains general-purpose. The tool does a `get_task` first to read current status, validates, then calls the update.
- **Terminal states are final.** `completed` and `cancelled` have no outbound transitions. This matches `validate_work_item()` which already treats them as non-active.
- **`blocked → in_progress` allowed** (the un-block case). `blocked → pending` is not — if unblocked, resume work, don't regress.
- **Clear error messages** include the allowed transitions: `"Cannot transition from 'completed' to 'in_progress'. 'completed' is a terminal state."`

### 2. New `check_work_item` Tool

Unit struct registered in `default_tools()`. Reads work item details and optionally fetches linked GitHub PR/issue status.

Architecture decisions:
- **Follows `brave_api_key` pattern** — added `github_token: Option<&'a str>` to `ToolContext`, threaded through all ~20 construction sites. Reuses existing `MIKA_INVESTIGATE_GITHUB_TOKEN` config key.
- **One-shot `reqwest::Client`** per invocation (no client in ToolContext). Acceptable because `check_work_item` is user-initiated, not high-frequency.
- **URL parsing prevents SSRF.** `parse_github_ref()` extracts `owner/repo/number` from the URL, then calls `https://api.github.com/repos/{owner}/{repo}/pulls/{number}` — never fetches the raw `reference_url`.
- **Graceful degradation:** no token → skip API call with note; API error → return work item data with error note; non-GitHub URL → report as-is.
- **15s timeout override** via `timeout_secs()`.

### 3. System Prompt Two-Pattern Guidance

Updated conversation mode prompt to teach the agent two interaction patterns:
- **Direct update:** User says "mark it done" → agent calls `update_work_item_status` directly
- **Inspect first:** User says "check the task" → agent calls `check_work_item`, presents findings, waits for confirmation

## Prevention

- **When adding new status values to any state machine**, grep for ALL SQL queries that reference adjacent statuses (lesson from `failed-callback-tasks-silently-dropped.md`). Check SELECTs, UPDATEs, partial index predicates, error-recovery paths, and display/framing code.
- **When threading a new field through `ToolContext`**, follow the `brave_api_key` pattern exactly — search for all `brave_api_key` occurrences and add the matching field at each site. The compiler enforces struct completeness, but the pattern ensures consistency.
- **For tools that fetch external URLs**, never follow the raw URL. Parse it to extract structured data, then construct the API call URL programmatically. This prevents SSRF.

## Cross-References

- `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — prevention checklist for agent-facing tools
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — code guard pattern, `validate_work_item()` active status check
- `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — why all status query paths must be audited when changing a state machine
- `docs/solutions/architecture-patterns/conditional-investigation-tool-registration.md` — GitHub API call pattern (reqwest, error mapping)
