---
title: "fix: Allow metadata-only writes on terminal-state tasks in update_work_item_status"
type: fix
status: active
date: 2026-04-18
---

# fix: Allow metadata-only writes on terminal-state tasks in update_work_item_status

## Overview

When `update_work_item_status` receives an invalid status transition but the caller also provided metadata, the entire call is rejected — metadata included. This loses late-arriving callback metadata (cost, duration, PR URL) on tasks that were already completed by a faster structural handler. The fix adds a metadata-only fallback path for terminal-state tasks.

## Problem Frame

Two legitimate code paths race to update the same task:
1. **verdict_handler** (structural, Rust) — fast, completes the task on QA pass + CI green
2. **Callback handler** (LLM-driven, ~60s later) — tries to persist `claude_pilot` metadata with `status: "in_progress"`

The structural handler wins the race, marking the task `completed`. The callback handler's call is rejected because `completed → in_progress` is invalid. The metadata (cost, duration, PR URL, session ID) is lost.

## Requirements Trace

- R1. `update_work_item_status(task_id, status="in_progress", metadata={...})` on a `completed` task → metadata applied, status unchanged, success response
- R2. `update_work_item_status(task_id, status="pending", metadata={...})` on a `completed` task → rejected (genuinely invalid backward transition — `pending` is before `in_progress` in the lifecycle)
- R3. `update_work_item_status(task_id, status="completed", metadata={...})` on a `completed` task → metadata applied (same-status path, already works)
- R4. Status-only calls to terminal-state tasks without metadata → rejected as today (no behavior change)
- R5. Phantom retry guard must still fire before the new path

## Scope Boundaries

- No schema change
- No new tool or DB function — reuses existing `merge_and_persist_metadata()`
- No prompt change
- No changes to `complete_task` or `cancel_task` tools
- Tool description update to document the new behavior

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/update_work_item_status.rs` — entire tool implementation
- Lines 193–204: same-status short-circuit (already applies metadata and returns success)
- Lines 206–221: transition validation (the rejection point to modify)
- Lines 269–294: `merge_and_persist_metadata()` helper (reusable)
- `allowed_transitions()` returns `&[]` for terminal states — usable to detect terminal vs non-terminal rejection

### Institutional Learnings

- **Phantom retry guard** must be preserved — it runs before transition validation and blocks retry-semantic metadata on active dispatches
- **Two-level shallow merge** via `merge_metadata()` in `work_item_metadata.rs` must be used for all metadata writes
- **Dispatch readiness guard** is separate from status transitions — metadata writability must not affect dispatch eligibility
- **Callback loop prevention** — the fix must not create a path to re-activate completed work items

## Key Technical Decisions

- **Terminal-state + metadata = apply metadata, skip status change**: When the transition is invalid AND the current status is terminal AND metadata is provided, apply the metadata and return success with an informational message. This is the narrowest possible relaxation.
- **Non-terminal invalid transitions still fully rejected**: If the current status is NOT terminal (e.g., `in_progress → pending`), the call is rejected entirely even if metadata is provided. This preserves the state machine's forward-only constraint for non-terminal states.
- **No audit event for metadata-only path**: Consistent with the same-status short-circuit (lines 193–204) which also skips audit events. Metadata writes are observable through the task's metadata field directly.

## Implementation Units

- [x] **Unit 1: Add terminal-state metadata fallback in transition validation**

**Goal:** When transition validation fails on a terminal-state task and metadata is provided, apply metadata and return success instead of an error.

**Requirements:** R1, R2, R4, R5

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/update_work_item_status.rs`

**Approach:**
- In the `!is_valid_transition()` branch (lines 207–221), before returning the error, check two conditions: (a) `allowed.is_empty()` (terminal state) AND (b) `metadata_input.is_some()`
- If both true: call `merge_and_persist_metadata()`, return `ToolOutput::success(...)` with message like `"Status unchanged ('{old_status}' is terminal). Metadata updated."`
- If metadata is None on a terminal state: reject as today (R4)
- If not terminal (allowed is non-empty): reject as today
- Update the tool description string to mention that metadata can be applied to terminal-state tasks

**Patterns to follow:**
- Same-status short-circuit at lines 193–204 — identical pattern of "apply metadata, return success with informational message"

**Test scenarios:**
- Happy path: `completed` task + `status="in_progress"` + metadata → success, metadata persisted, status still `completed`
- Happy path: `cancelled` task + `status="in_progress"` + metadata → success, metadata persisted, status still `cancelled`
- Happy path: metadata merges with existing metadata on terminal task (two-level shallow merge preserved)
- Edge case: `completed` task + `status="in_progress"` + NO metadata → rejected as terminal state (R4 — no behavior change)
- Edge case: `cancelled` task + `status="pending"` + metadata → rejected as terminal state (same treatment — all transitions from terminal states are invalid, so any non-same-status call with metadata gets the fallback)
- Error path: `in_progress` task + `status="pending"` + metadata → rejected as invalid transition (non-terminal, no fallback)
- Integration: phantom retry guard still fires before the terminal metadata path — `completed` task with active callback child + retry-semantic metadata → rejected by phantom guard

**Verification:**
- All existing tests pass (with updates to `test_terminal_state_cannot_transition`)
- New test cases cover all acceptance criteria from the issue
- `cargo clippy` clean

- [x] **Unit 2: Update existing terminal-state test**

**Goal:** Update `test_terminal_state_cannot_transition` to account for the new metadata fallback behavior.

**Requirements:** R1, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/update_work_item_status.rs` (test module)

**Approach:**
- The existing test iterates all non-same-status targets from terminal states and asserts error. Status-only calls (no metadata) should still error — keep those assertions.
- The test does NOT currently pass metadata, so it should continue to pass as-is for R4 (status-only calls still rejected)
- Verify the test still passes without modification; if it does, this unit is a no-op

**Test expectation:** Verify existing test compatibility — no new test code if the existing test already covers R4.

**Verification:**
- `cargo test -p mika-agent -- update_work_item_status` passes

## System-Wide Impact

- **Interaction graph:** The phantom retry guard (step 3) runs before transition validation (step 5), so it is unaffected. The dispatch readiness guard in `skills/executor.rs` is a separate code path and is unaffected.
- **Error propagation:** The new path returns `ToolOutput::success`, not an error. Late-arriving callbacks will no longer lose metadata.
- **State lifecycle risks:** None — the task's status is never changed by the new path. Only the metadata column is written. `completed_at` is unaffected.
- **API surface parity:** The `complete_task` and `cancel_task` tools have their own paths and are unaffected.
- **Unchanged invariants:** Terminal states remain terminal. No status transition from `completed` or `cancelled` is ever permitted. The state machine in `VALID_TRANSITIONS` is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| LLM uses the metadata fallback to avoid status errors without providing useful metadata | Low risk — the fallback only fires when metadata is actually provided. Empty/null metadata still gets the error. |

## Sources & References

- Related issue: #617
- Related issues: #608 (task vocabulary refactor), #609 (milestone callback routing)
- Existing code: `crates/mika-agent/src/tools/update_work_item_status.rs`
- Learnings: `docs/solutions/architecture-patterns/work-item-status-transition-validation.md`
- Learnings: `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md`
