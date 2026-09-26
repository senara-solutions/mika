---
title: "fix: Split TUI [N running] badge into executing vs queued counts"
type: fix
status: active
date: 2026-05-12
issue: "#1057"
---

# fix: Split TUI [N running] badge into executing vs queued counts

## Overview

The TUI footer badge `[N running]` conflates actively-executing callback tasks (claude-pilot subprocess running) with queued/deferred wrappers (acknowledged but waiting for a dispatch slot). This causes operator misreads — `[3 running]` implies parallel execution when only 1 subprocess is active and 2 are waiting. The fix splits the counter into `[1 running, 2 queued]` (Option A from the ticket), preserving queue-depth visibility while making execution state clear at a glance.

## Problem Frame

When mika-dev dispatches multiple sequential grooms, the TUI shows `[3 running]` because `get_active_background_task_count()` counts all callback tasks with `status IN ('pending', 'in_progress')` regardless of whether they have a live subprocess. The operator cannot distinguish executing from queued without running `mika tasks list` and checking PID annotations.

## Requirements Trace

- R1. Operator can distinguish "actively executing" from "queued for next slot" at a glance from the TUI
- R2. Counter shape is consistent between TUI status bar and `mika tasks list` output
- R3. No behavior change to dispatch sequencing — purely a display fix

## Scope Boundaries

- No changes to task state machine or dispatch logic
- No schema migration — `process_id` column already exists
- No changes to `mika tasks list` row format — it already shows `[PID N]` suffixes which serve R2

### Deferred to Separate Tasks

- `mika tasks list` header could show a summary like `Active Tasks (1 running, 2 queued):` instead of `Active Tasks (3):` — cosmetic enhancement, not blocking

## Pinned Source (Phase 0)

### Current `get_active_background_task_count()` body

```sql
-- db.rs:5129-5133
SELECT COUNT(*) FROM tasks
WHERE agent_id = ?1
  AND trigger_type = 'callback'
  AND action_type = 'resume_agent'
  AND status IN ('pending', 'in_progress')
```

Returns `Result<usize>`. Async wrapper at `async_db.rs:424-426` delegates via `with_db`.

### Exhaustive caller inventory

| Caller | File | Usage |
|--------|------|-------|
| `App::tick()` | `crates/mika-cli/src/tui/app.rs:1035` | Polls every ~5s, stores in `self.active_background_task_count` |
| `AsyncDatabase::get_active_background_task_count()` | `crates/mika-agent/src/async_db.rs:424` | Async wrapper (sole intermediary) |

No server, engine, dispatch, or skill code calls this function. The async wrapper is the sole consumer, called only from `app.rs` tick. Three existing test functions in `db.rs:10921-10988` exercise the sync method directly.

### `process_id` lifecycle

| Event | Code site | Effect |
|-------|-----------|--------|
| **Set** | `skills/executor.rs:1424-1425` — `spawn_long_running_exec()` | `set_task_process_id(&task_id, Some(pid))` immediately after `child.id()` |
| **Cleared (cancel)** | `task_engine/process_kill.rs:176` — `cancel_task_and_kill()` | `clear_task_process_id()` after `kill_process_gracefully()` |
| **Cleared (orphan kill)** | `task_engine/engine.rs:305-306` — `kill_orphan_processes()` | `clear_task_process_id()` after `kill_process_immediate()` for expired tasks |
| **Never explicitly cleared on normal completion** | Callback delivery marks task `completed`/`delivered` but does NOT clear `process_id` | Stale `process_id` on completed tasks is harmless — our query filters `status IN ('pending', 'in_progress')` |

**Stale-PID invariant:** Between subprocess death and watchdog cleanup (~60s detection + 120s grace = ~3 min), `process_id` remains set on a dead-process task. The old counter also counted these as "running." The new discriminator (`process_id IS NOT NULL AND status = 'in_progress'`) is strictly more accurate: it excludes completed/failed tasks with stale `process_id`. The stale-PID window is inherited, not widened.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs:5127-5138` — `get_active_background_task_count()` current COUNT query
- `crates/mika-agent/src/async_db.rs:424-426` — async wrapper
- `crates/mika-cli/src/tui/app.rs:581` — `active_background_task_count: usize` field
- `crates/mika-cli/src/tui/app.rs:1032-1040` — polling in `tick()`
- `crates/mika-cli/src/tui/ui.rs:1063-1070` — badge rendering in `draw_footer()`
- `crates/mika-cli/src/tui/commands/handlers.rs` — `/clear` preserves background task count (agent-scoped)
- Existing pattern: `pending_task_count` + `active_background_task_count` — same polling/rendering shape

### Institutional Learnings

- `docs/solutions/ux-improvements/tui-background-task-running-indicator.md` — original badge implementation; documents the agent-scoped vs session-scoped semantics and `/clear` preservation
- `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — warns that callback status filters across db.rs must be kept consistent; audit all callback query locations when adding new filters
- `docs/solutions/logic-errors/callback-processing-race-steals-tui-notifications.md` — TUI polling and engine dispatch share callback queries; new filters must not interfere with claim ordering

## Key Technical Decisions

- **Option A (split counter) over Option B (hide queued):** Preserves queue-depth visibility which is valuable for reasoning about dispatch backlog and cost
- **`process_id IS NOT NULL` as the executing signal:** The `process_id` column is populated by `spawn_long_running_exec()` at subprocess spawn time and used by the callback watchdog (#959) for liveness checks. It is the ground-truth indicator that a subprocess is running — more reliable than `status = 'in_progress'` alone (which is set when mika-dev accepts the dispatch, before spawn)
- **Return a struct from one query, not two queries:** Single `SELECT` with conditional counting (`SUM(CASE WHEN ... END)`) avoids doubling DB round-trips on the 5s polling cadence
- **Replace single field with a struct:** `BackgroundTaskCounts { executing: usize, queued: usize }` replaces `active_background_task_count: usize` — clearer than two loosely-coupled fields

## Open Questions

### Resolved During Planning

- **Should deferred callbacks (label `LIKE '%:deferred'`) count as queued or executing?** Queued — they have no subprocess and are waiting for a dispatch slot, same as pending wrappers. The `process_id IS NULL` filter naturally classifies them correctly.
- **Should we change `mika tasks list` row output?** No — it already shows `[PID N]` for executing tasks, which satisfies R2. The header line could be enhanced but that is a cosmetic follow-up.

### Deferred to Implementation

- Exact Rust struct field names — will follow existing naming conventions

## Implementation Units

- [ ] **Unit 1: Split the DB query**

**Goal:** Replace `get_active_background_task_count()` with a method that returns both executing and queued counts from a single query.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs`
- Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)]` module)

**Approach:**
- Add a `BackgroundTaskCounts` struct with `executing: usize` and `queued: usize` fields
- Replace `get_active_background_task_count()` with `get_background_task_counts()` returning `BackgroundTaskCounts`
- Single SQL query using conditional aggregation: executing = `process_id IS NOT NULL AND status = 'in_progress'`; queued = the remainder (`process_id IS NULL` or `status = 'pending'`)
- Keep the same base filter: `trigger_type = 'callback' AND action_type = 'resume_agent' AND status IN ('pending', 'in_progress')`
- Update the async wrapper in `async_db.rs` to match

**Patterns to follow:**
- Existing `get_active_background_task_count()` shape
- Other struct-returning db methods (e.g., `Task` struct pattern)

**Test scenarios:**
- Happy path: 1 task with process_id + 2 tasks without process_id -> `BackgroundTaskCounts { executing: 1, queued: 2 }`
- Happy path: 0 callback tasks -> `BackgroundTaskCounts { executing: 0, queued: 0 }`
- Edge case: all tasks executing (all have process_id) -> queued = 0
- Edge case: all tasks queued (none have process_id) -> executing = 0
- Edge case: pending status tasks always count as queued regardless of process_id presence

**Verification:**
- `cargo test -p mika-agent` passes with new tests covering the split counts

- [ ] **Unit 2: Update TUI app state and polling**

**Goal:** Replace the single `active_background_task_count` field with executing + queued counts and update the polling logic.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/tui/app.rs`

**Approach:**
- Replace `active_background_task_count: usize` with two fields: `executing_task_count: usize` and `queued_task_count: usize`
- Update initialization (two locations) to set both to 0
- Update the tick polling block to call `get_background_task_counts()` and compare both values for change detection
- Preserve agent-scoped semantics: NOT reset on `/clear` (same as current behavior)

**Patterns to follow:**
- Existing polling pattern at `app.rs:1032-1040`
- Existing `/clear` preservation logic in `commands/handlers.rs`

**Test scenarios:**
- Happy path: verify `/clear` handler does NOT reset `executing_task_count` or `queued_task_count` (agent-scoped semantics preserved — same as existing `active_background_task_count` behavior documented in `docs/solutions/ux-improvements/tui-background-task-running-indicator.md`)

**Verification:**
- `cargo build` succeeds with no unused-field warnings
- `/clear` handler code inspection confirms new fields are NOT listed among reset fields

- [ ] **Unit 3: Update TUI footer badge rendering**

**Goal:** Render `[1 running, 2 queued]` instead of `[3 running]` in the TUI footer.

**Requirements:** R1, R2

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-cli/src/tui/ui.rs`

**Approach:**
- Update the badge rendering block at `ui.rs:1063-1070`
- Three display states:
  - Both > 0: `[N running, M queued]` — "running" in Yellow, "queued" in DarkGray or Cyan
  - Only executing > 0: `[N running]` — Yellow (same as current for single dispatch)
  - Only queued > 0: `[M queued]` — DarkGray (no active execution, tasks waiting)
  - Both = 0: no badge (same as current)
- Use multiple styled `Span`s within the badge for color differentiation

**Patterns to follow:**
- Existing multi-span badge patterns in `draw_footer()` (e.g., `[N tasks]` badge)
- Color conventions: Yellow for active/running, DarkGray for passive/waiting

**Test scenarios:**
- Test expectation: none — pure rendering logic. Visual correctness verified by running the TUI.

**Verification:**
- `cargo build` succeeds
- Running `mika` with active background tasks shows the split counter format

- [ ] **Unit 4: Update any remaining references**

**Goal:** Fix all compilation errors from the renamed field and ensure consistency.

**Requirements:** R1

**Dependencies:** Unit 3

**Files:**
- Modify: `crates/mika-cli/src/tui/commands/handlers.rs` (if `/clear` handler references the old field name)
- Grep: any other references to `active_background_task_count` across the codebase

**Approach:**
- Search for all references to `active_background_task_count` and update them
- The `/clear` handler intentionally does NOT reset this field — preserve that semantic with the new field names
- Verify no other crate consumes `get_active_background_task_count()` from the public API

**Patterns to follow:**
- Existing field reference patterns

**Test scenarios:**
- Test expectation: none — mechanical rename, no behavioral change

**Verification:**
- `cargo build` succeeds with zero warnings
- `cargo clippy` clean
- `cargo test` passes

## System-Wide Impact

- **Interaction graph:** The DB query is consumed only by TUI polling (app.rs tick). No server, engine, or skill code calls `get_active_background_task_count()`. The async wrapper is the sole consumer from CLI code.
- **Error propagation:** No change — query errors are absorbed by the `let Ok(...)` guard in the polling block.
- **State lifecycle risks:** None — the fields are display-only counters refreshed every 5s. No cache invalidation or write-back.
- **API surface parity:** `mika tasks list` already shows PID info per-row; no format change needed for R2 consistency. The `--format json` output includes `process_id` in the task JSON.
- **Unchanged invariants:** Dispatch sequencing, callback lifecycle, watchdog behavior, `/clear` agent-scoped semantics — all unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `process_id` may be NULL for a brief window between task creation and subprocess spawn | This is correct behavior — the task IS queued during that window. The display accurately reflects reality. |
| Stale `process_id` on dead subprocess (~3 min watchdog window) | The stale-PID window is inherited from the existing counter, not widened. The new discriminator is strictly more accurate because it excludes completed/failed tasks with stale `process_id`. No regression. |
| Other code paths consuming `get_active_background_task_count()` break | Exhaustive caller inventory (see Pinned Source): only the async wrapper calls it, called only from `app.rs` tick. Two callers total. |

## Sources & References

- Related issue: [#1057](https://github.com/senara-solutions/mika/issues/1057)
- Prior implementation: `docs/solutions/ux-improvements/tui-background-task-running-indicator.md`
- Callback audit learnings: `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md`
