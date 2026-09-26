---
title: "feat: Work-item status guard on run_claude_pilot dispatch"
type: feat
status: active
date: 2026-04-13
issue: 525
---

# feat: Work-item status guard on run_claude_pilot dispatch

## Overview

Add a hard, tool-level refusal in `execute_long_running()` (the code path for `run_claude_pilot` and all long-running exec-handler skills) that validates work item status and detects active child dispatches before spawning a subprocess. This prevents wasted compute from redundant dispatches when a work item is already being worked on, has an open PR, or is in a terminal state.

## Problem Frame

On 2026-04-11, mika-dev called `run_claude_pilot` with a fabricated UUID (hallucinated during recovery from a misclassified webhook). The tool returned "Work item not found." mika-dev then retried with the correct task_id and successfully dispatched a new claude-pilot session on a work item that was already `in_progress` with PR #522 open, QA-approved, and CI green. The redundant session burned ~$0.18 producing no useful output and contributed to a desync that left the PR stuck for 7 hours.

The root cause: `execute_long_running()` delegates all dispatch-readiness decisions to the LLM via soft advisory returns from `create_work_item`. Neither the LLM judgment nor the advisory string is sufficient against misclassified webhooks + hallucinated UUIDs + LLM recovery improvisation. The tool boundary is the only reliable enforcement point.

## Requirements Trace

- R1. `execute_long_running()` validates `task_id` against the database before launching the subprocess
- R2. Reject dispatch when work item status is `blocked`, `completed`, or `cancelled` with a structured error including `error`, `task_id`, `current_status`, `pr_url`, and `reason` fields
- R3. Reject fabricated/nonexistent `task_id` with a distinct `work_item_not_found` error (defense-in-depth — already partially covered by `validate_work_item()`)
- R4. Only `pending` and `in_progress` are valid pre-dispatch statuses; all others (`blocked`, `completed`, `cancelled`) are rejected per R2
- R5. Reject dispatch when the work item already has an active callback child task (double-dispatch prevention)
- R6. Unit tests for each rejected status, fabricated task_id, and double-dispatch scenario
- R7. Auto-transition `pending` work items to `in_progress` on successful dispatch (closes the bypass window where two dispatches to a `pending` item both pass)

## Scope Boundaries

- Guard applies to ALL long-running exec-handler dispatches, not just `run_claude_pilot` — `execute_long_running()` is the shared entry point
- The shared `validate_work_item()` helper in `tools/mod.rs` is NOT modified — it is co-consumed by `delegate_task` which intentionally allows `blocked` work items
- No new task statuses (e.g., `in_pr`, `merging`, `merged`) are added — the guard uses the existing status model plus child-task introspection
- No schema migration — uses existing `get_child_tasks()` query and existing `update_manual_task_status()` for the auto-transition

### Deferred to Separate Tasks

- Structural verdict handler for the webhook side of the same vulnerability: #524 (separate PR, already planned)
- Composite index on `parent_task_id + trigger_type + status` for the child-task query: performance optimization, separate PR if profiling shows need

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/executor.rs:552` — `execute_long_running()`, the insertion point for the guard. Currently calls `validate_work_item()` at line 565
- `crates/mika-agent/src/tools/mod.rs:234` — `validate_work_item()` accepts `pending | in_progress | blocked`; shared with `delegate_task`
- `crates/mika-agent/src/tools/update_work_item_status.rs:8-26` — `VALID_STATUSES` and `VALID_TRANSITIONS` state machine
- `crates/mika-agent/src/async_db.rs:465` — `get_child_tasks(parent_task_id)` returns `Vec<Task>`
- `crates/mika-agent/src/db.rs:3024` — `update_manual_task_status()` for the auto-transition
- `crates/mika-agent/src/task_engine/types.rs:1-15` — `task_status` constants
- `crates/mika-agent/src/skills/executor.rs:1367` — existing long-running tests with `TestHarness`/`LongRunningContext` pattern

### Institutional Learnings

- **Code guards over prompt instructions** (docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md): "If the agent ignoring an instruction would cause real harm, enforce it in the harness." Dispatch of a redundant claude-pilot session causes real harm.
- **Delegation work item guard** (docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md): Establishes the three-layer defense pattern (code guard + core memory + system prompt). The code guard is the primary enforcement; prompt-level is defense-in-depth.
- **Tool-layer validation, not DB-layer** (docs/solutions/architecture-patterns/work-item-status-transition-validation.md): Validation belongs in the tool/executor layer; the DB stays general-purpose.
- **Atomic check + action** (docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md): `pr_merge_with_gate` demonstrates the pattern of checking preconditions and acting in one tool call.
- **Silent success is a bug** (docs/solutions/logic-errors/long-running-monitor-false-failure-on-signal.md): Rejected dispatches must return explicit structured errors, not silently succeed.

## Key Technical Decisions

- **Separate dispatch validation instead of modifying shared helper:** `validate_work_item()` is shared with `delegate_task` which allows `blocked` status. Adding a `validate_work_item_for_dispatch()` helper (or inline validation in `execute_long_running`) avoids changing delegation behavior. The existing `validate_work_item()` call remains as the first-pass check; the new guard is a stricter second pass.

- **Structured JSON error in `ToolOutput::error()` content field:** The issue requests structured JSON errors. While existing tools use plain-text errors, the JSON structure provides programmatic feedback to the LLM. The content field of `ToolOutput::error()` will contain a JSON string. This matches `pr_merge_with_gate`'s pattern of returning machine-readable results.

- **Active-child detection via application-level filtering:** Query `get_child_tasks(work_item_id)` and filter for `trigger_type == "callback" && status IN (pending, in_progress)` in Rust. This avoids adding a new DB query and keeps the filtering logic colocated with the guard.

- **Auto-transition to `in_progress` on dispatch:** When the guard passes and the work item is `pending`, auto-transition to `in_progress` before creating the callback task. This closes the TOCTOU window for double-dispatch on `pending` items. Intra-turn tool calls are sequential in the agent loop, so the second call would see the `in_progress` status and the active child.

- **Treat `expired`, `failed`, `completed`, `cancelled`, `delivered` callback children as inactive:** Only `pending` and `in_progress` callback children block dispatch. This allows legitimate retries after failures/timeouts.

## Open Questions

### Resolved During Planning

- **Should `validate_work_item()` be modified?** No — it's shared with `delegate_task` which intentionally allows `blocked`. Add stricter validation after the shared check in `execute_long_running()`.
- **Are `in_pr`/`merging`/`merged` real statuses?** No — the issue uses conceptual names. The actual work item statuses are `pending`, `in_progress`, `blocked`, `completed`, `cancelled`. The guard maps the issue's intent to: reject `blocked`/`completed`/`cancelled`; allow `pending`/`in_progress`; detect active children for double-dispatch.
- **Is the intra-turn race a concern?** Minimal — tool calls within a turn execute sequentially in the agent loop. The auto-transition to `in_progress` + callback child creation before the second call runs closes this window.

### Deferred to Implementation

- Exact error message wording — directional in the plan, finalized during implementation
- Whether to emit an audit event for rejected dispatches — low stakes, decide during implementation

## Implementation Units

- [x] **Unit 1: Add dispatch-readiness validation in `execute_long_running()`**

  **Goal:** Reject long-running dispatch when the work item is in a non-dispatchable status or already has an active callback child task.

  **Requirements:** R1, R2, R3, R4, R5

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/skills/executor.rs`
  - Test: `crates/mika-agent/src/skills/executor.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - After the existing `validate_work_item()` call (line 565), add a second validation pass:
    1. Re-fetch the task via `ctx.db.get_task(work_item_id)` (the shared helper already confirmed existence; we need the full `Task` struct for status + metadata)
    2. Check status is `pending` or `in_progress` — if not, return `ToolOutput::error()` with JSON `{"error": "work_item_not_dispatchable", "task_id": "...", "current_status": "...", "pr_url": "..." or null, "reason": "..."}`
    3. Query `ctx.db.get_child_tasks(work_item_id)` and filter for active callback children (`trigger_type == "callback" && status in ["pending", "in_progress"]`)
    4. If any active callback children exist, return `ToolOutput::error()` with JSON `{"error": "work_item_active_dispatch", "task_id": "...", "active_child_id": "...", "reason": "..."}`
  - Extract `pr_url` from the task's `metadata` JSON field (parse `metadata` string, look for `claude_pilot.pr_url` or `pr_url` key)
  - The existing `validate_work_item()` already handles the `work_item_not_found` case (R3) — no change needed there

  **Patterns to follow:**
  - `crates/mika-agent/src/skills/executor.rs:565` — existing `validate_work_item()` call site
  - `crates/mika-agent/src/tools/update_work_item_status.rs:107-128` — inline validation before action
  - `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — structured JSON in `ToolOutput` content

  **Test scenarios:**
  - Happy path: work item `pending`, no children → dispatch proceeds (no error returned from validation)
  - Happy path: work item `in_progress`, only `completed`/`failed`/`expired` callback children → dispatch proceeds
  - Error path: work item `blocked` → `work_item_not_dispatchable` error with `current_status: "blocked"`
  - Error path: work item `completed` → rejected (already caught by `validate_work_item()`, verify error message)
  - Error path: work item `cancelled` → rejected (already caught by `validate_work_item()`, verify error message)
  - Error path: nonexistent task_id → `work_item_not_found` error (verify existing behavior preserved)
  - Error path: empty task_id → existing "create a work item first" error preserved
  - Edge case: work item `in_progress` with one `pending` callback child → `work_item_active_dispatch` error including active child ID
  - Edge case: work item `in_progress` with one `in_progress` callback child → `work_item_active_dispatch` error
  - Edge case: work item `in_progress` with mixed children (one `completed` callback, one `pending` callback) → rejected (active child exists)
  - Edge case: work item `in_progress` with non-callback children only (e.g., delegate tasks) → dispatch proceeds (only callback children block)

  **Verification:**
  - All long-running dispatch attempts against non-dispatchable work items return structured error JSON
  - Existing happy-path test (`test_long_running_creates_callback_task`) still passes
  - Existing missing-work-item test still passes

- [x] **Unit 2: Auto-transition pending work items to `in_progress` on dispatch**

  **Goal:** Close the TOCTOU window where two dispatches to a `pending` work item both pass the guard by auto-transitioning to `in_progress` before creating the callback task.

  **Requirements:** R7

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/skills/executor.rs`
  - Test: `crates/mika-agent/src/skills/executor.rs` (inline tests)

  **Approach:**
  - After the dispatch-readiness validation passes and before creating the callback `NewTask`, check if the work item status is `pending`
  - If `pending`, call `ctx.db.update_manual_task_status(work_item_id, "in_progress")` to transition atomically (two-arg async wrapper; `agent_id` is injected internally)
  - Log the auto-transition at `info` level with the work_item_id
  - If the status update fails (e.g., concurrent modification), continue anyway — the callback child creation provides the secondary guard

  **Patterns to follow:**
  - `crates/mika-agent/src/tools/update_work_item_status.rs` — status transition via `update_manual_task_status`
  - `crates/mika-agent/src/skills/executor.rs:569` — existing code between validation and callback creation

  **Test scenarios:**
  - Happy path: dispatch with `pending` work item → work item transitions to `in_progress` before callback creation; verify status via `db.get_task()`
  - Happy path: dispatch with `in_progress` work item → no status change attempted
  - Integration: two sequential dispatches to same `pending` work item → first succeeds and transitions to `in_progress`; second is rejected by active-child check (callback task from first dispatch exists)

  **Verification:**
  - Work item is `in_progress` after successful dispatch regardless of initial status
  - Double-dispatch to a `pending` work item is prevented by the combination of auto-transition and active-child detection

- [x] **Unit 3: Integration test — replay the #522 race scenario**

  **Goal:** Prove the guard prevents the exact failure mode from the PR #522 incident: dispatching claude-pilot on a work item that already has an active session.

  **Requirements:** R6

  **Dependencies:** Unit 1, Unit 2

  **Files:**
  - Modify: `crates/mika-agent/src/skills/executor.rs` (inline tests)

  **Approach:**
  - Create a work item in `in_progress` status
  - Create a callback child task with `trigger_type=callback` and `status=pending` (simulating an active claude-pilot session)
  - Optionally set `metadata` with `claude_pilot.pr_url` to simulate the PR-open state
  - Call `execute_skill_tool()` with the same `work_item_id`
  - Assert `is_error == true` and content contains `work_item_active_dispatch`
  - Assert no new callback task was created (count tasks before and after)

  **Patterns to follow:**
  - `crates/mika-agent/src/skills/executor.rs:1417` — `test_long_running_creates_callback_task` pattern for setting up work items and invoking `execute_skill_tool`

  **Test scenarios:**
  - Integration: work item `in_progress` with active callback child + PR URL in metadata → dispatch rejected with structured error containing `pr_url`
  - Integration: work item `in_progress` with active callback child, no metadata → dispatch rejected, `pr_url` is null in error
  - Integration: after the active callback child transitions to `completed`, a retry dispatch succeeds

  **Verification:**
  - The exact PR #522 failure mode is prevented
  - No subprocess is spawned when the guard rejects
  - Retry after child completion works correctly

## System-Wide Impact

- **Interaction graph:** `execute_long_running()` is the shared entry point for all long-running exec-handler skills. The guard affects `run_claude_pilot` and any future long-running tools. `delegate_task` is unaffected (uses `validate_work_item()` directly, not `execute_long_running()`).
- **Error propagation:** Structured JSON errors propagate to the LLM as `ToolOutput::error()`. The LLM sees the error in the tool result and should adjust behavior accordingly.
- **State lifecycle risks:** The auto-transition from `pending` to `in_progress` is a side effect of dispatch. If dispatch fails after the transition (e.g., subprocess spawn failure), the work item remains `in_progress` — this is acceptable because the agent would naturally notice and handle it.
- **API surface parity:** No other interfaces dispatch long-running tools. The HTTP server's `/tasks/{id}/complete` endpoint is for callback completion, not dispatch.
- **Unchanged invariants:** `validate_work_item()` in `tools/mod.rs` is unchanged. `delegate_task` behavior is unchanged. The `Task` struct, DB schema, and status constants are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Modifying `execute_long_running()` could break existing long-running skill dispatches | Existing tests (`test_long_running_creates_callback_task`, `test_long_running_missing_work_item_id`) serve as regression guards; run full test suite |
| Auto-transition side effect changes work item lifecycle | Aligns with expected behavior — dispatching work naturally means it's "in progress." The `update_work_item_status` tool already validates transitions. |
| `get_child_tasks()` performance on work items with many children | Bounded in practice — work items rarely have more than a handful of children. Deferred index optimization to separate task. |

## Sources & References

- Related issue: #525
- Related PR incident: #522 (stuck for 7 hours due to redundant dispatch)
- Companion ticket: #524 (structural verdict handler — webhook side)
- Compound doc: `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md`
- Delegation guard pattern: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
