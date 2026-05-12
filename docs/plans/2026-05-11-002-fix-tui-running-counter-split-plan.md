---
title: "fix: Split TUI running counter into executing + queued"
type: fix
status: active
date: 2026-05-11
---

# fix: Split TUI running counter into executing + queued

## Overview

The TUI footer badge `[N running]` conflates actively-executing callback tasks (subprocess alive) with queued/deferred ones (waiting for dispatch slot). This caused an operator misread (2026-05-10) where `[3 running]` implied parallel execution when only 1 claude-pilot was running and 2 were deferred behind it. Fix by splitting the counter using the existing `process_id` field as the discriminator.

## Problem Frame

`get_active_background_task_count()` counts all callback+resume_agent tasks with `status IN ('pending', 'in_progress')` regardless of whether a subprocess is actually running. The `process_id` column already tracks live subprocesses (set on spawn, cleared on death), but the TUI counter ignores it. The same conflation exists in `mika tasks list` output, which shows raw status without distinguishing executing from queued.

## Requirements Trace

- R1. Operator can distinguish "actively executing" from "queued for next slot" at a glance from the TUI
- R2. Counter shape is consistent between TUI status bar and `mika tasks list` output
- R3. No behavior change to dispatch sequencing — purely display fix

## Scope Boundaries

- No changes to task state machine, dispatch logic, or callback lifecycle
- No new DB columns or migrations
- No changes to `--format json` output shapes (only human-readable text)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs:4947-4958` — `get_active_background_task_count()` current query
- `crates/mika-agent/src/db.rs:152` — `process_id: Option<i64>` on Task struct
- `crates/mika-agent/src/db.rs:5086` — `set_task_process_id()` sets PID on spawn
- `crates/mika-agent/src/db.rs:5118` — `get_active_callback_tasks_with_process_id()` already queries by PID presence
- `crates/mika-agent/src/async_db.rs:424-426` — async wrapper pattern
- `crates/mika-cli/src/tui/ui.rs:1064-1070` — footer badge rendering
- `crates/mika-cli/src/tui/app.rs:579-581` — `active_background_task_count` field + polling at line 1032
- `crates/mika-cli/src/commands/tasks.rs:87-102` — `print_task_summary()` already shows `[PID N]`
- `crates/mika-cli/src/tui/commands/handlers.rs:1630-1652` — `/clear` test preserves background count

## Key Technical Decisions

- **Use `process_id IS NOT NULL` as the executing discriminator**: The `process_id` field is already the source of truth for subprocess liveness — set by `spawn_long_running_exec()`, cleared by the callback watchdog on process death. No new heuristic needed.
- **Return a struct, not a tuple**: A `BackgroundTaskCounts { executing: usize, queued: usize }` struct is clearer than `(usize, usize)` at callsites and self-documents the meaning.
- **Keep the old method as a convenience**: `get_active_background_task_count()` can delegate to the new method and sum, preserving any callers and keeping tests simpler to migrate incrementally.

## Implementation Units

- [x] **Unit 1: DB + async layer — split counter query**

  **Goal:** Add `get_background_task_counts()` returning `BackgroundTaskCounts { executing, queued }` and update the async wrapper.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs`
  - Modify: `crates/mika-agent/src/async_db.rs`
  - Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)]` module)

  **Approach:**
  - Add `BackgroundTaskCounts` struct in `db.rs` near the existing method
  - New query uses two `SUM(CASE WHEN process_id IS NOT NULL THEN 1 ELSE 0 END)` expressions in a single SELECT to avoid two round-trips
  - Rewrite `get_active_background_task_count()` to delegate: call `get_background_task_counts()` and return `executing + queued`
  - Add async wrapper `get_background_task_counts()` in `async_db.rs` following the existing `with_db` pattern

  **Patterns to follow:**
  - Existing `get_active_background_task_count()` for query structure
  - Existing async wrappers in `async_db.rs` for the delegation pattern

  **Test scenarios:**
  - Happy path: Two pending callback tasks, one with `process_id` set, one without → `executing=1, queued=1`
  - Happy path: No callback tasks → `executing=0, queued=0`
  - Happy path: All tasks have `process_id` → `executing=N, queued=0`
  - Happy path: No tasks have `process_id` → `executing=0, queued=N`
  - Edge case: Task completed (terminal status) with `process_id` still set → not counted in either bucket
  - Integration: `get_active_background_task_count()` returns sum of both fields (backward compat)

  **Verification:**
  - `cargo test -p mika-agent -- test_get_active_background_task_count` passes (existing tests + new ones)

- [x] **Unit 2: TUI app state + footer badge**

  **Goal:** Split the `active_background_task_count` field into `executing_task_count` + `queued_task_count` and render `[1 running, 2 queued]` in the footer.

  **Requirements:** R1

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-cli/src/tui/app.rs`
  - Modify: `crates/mika-cli/src/tui/ui.rs`
  - Modify: `crates/mika-cli/src/tui/commands/handlers.rs` (test update)

  **Approach:**
  - Replace `active_background_task_count: usize` with `executing_task_count: usize` and `queued_task_count: usize`
  - Update polling block (~line 1032) to call `get_background_task_counts()` and set both fields
  - Update `/clear` handler to preserve both fields (agent-scoped, not session-scoped)
  - Badge rendering logic in `ui.rs`:
    - Both > 0: `[1 running, 2 queued]` (Yellow)
    - Only executing > 0: `[1 running]` (Yellow)
    - Only queued > 0: `[2 queued]` (Yellow)
    - Both 0: no badge (existing behavior)

  **Patterns to follow:**
  - Existing badge rendering for `[N tasks]` and `[N hidden]` in `ui.rs`
  - Existing `/clear` preservation test in `handlers.rs`

  **Test scenarios:**
  - Happy path: Update `/clear` test to assert both `executing_task_count` and `queued_task_count` are preserved
  - Edge case: Both counts zero → no badge rendered (existing behavior)

  **Verification:**
  - `cargo test -p mika-cli` passes
  - `cargo clippy -p mika-cli` clean

- [x] **Unit 3: `mika tasks list` — annotate callback task status**

  **Goal:** Add `[executing]` or `[queued]` annotation to callback tasks in `print_task_summary()` for consistency with the TUI badge.

  **Requirements:** R2

  **Dependencies:** None (uses existing `process_id` field on Task struct)

  **Files:**
  - Modify: `crates/mika-cli/src/commands/tasks.rs`

  **Approach:**
  - In `print_task_summary()`, for callback tasks (`trigger_type == "callback"`), replace the raw `[PID N]` suffix with `[executing, PID N]` when `process_id.is_some()`, or `[queued]` when `process_id.is_none()` and status is `pending` or `in_progress`
  - Non-callback tasks unchanged

  **Patterns to follow:**
  - Existing `pid_info` formatting in `print_task_summary()` at line 94-97

  **Test scenarios:**
  - Test expectation: none — pure formatting change on a display-only function; verified manually via `mika tasks list`

  **Verification:**
  - `cargo build -p mika-cli` compiles
  - Manual: `mika tasks list` shows `[executing, PID N]` for active callbacks and `[queued]` for deferred ones

## System-Wide Impact

- **Interaction graph:** Only TUI polling and CLI task list rendering are affected. No callbacks, middleware, or dispatch logic touched.
- **API surface parity:** `--format json` output for `mika tasks list` already includes `process_id` as a field; no change needed there.
- **Unchanged invariants:** Task state machine, dispatch sequencing, callback lifecycle, watchdog behavior all untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `process_id` may briefly be NULL between task creation and subprocess spawn | This is correct behavior — the task genuinely is "queued" during that window. The counter accurately reflects reality. |
| Subprocess dies but `process_id` not yet cleared by watchdog | Brief ~60s window where a dead task shows as "executing". Acceptable — the watchdog clears it on the next tick. No worse than current behavior. |

## Sources & References

- Related issue: senara-solutions/mika#1057
- Callback watchdog: mika#959 (process liveness detection via `process_id`)
- Deferred dispatch: mika#1058, mika#1070 (deferred callback promotion)
