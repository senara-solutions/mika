---
status: complete
priority: p3
issue_id: "494"
tags: [code-review, quality, database]
dependencies: []
---

# AsyncDatabase Has Dead Methods From Removed Scheduler

## Problem Statement

Several methods in `AsyncDatabase` are defined but have no callers outside their definition files.
These are remnants of the removed `ReminderScheduler` or redundant with existing methods:

- `get_due_tasks` — redundant with `get_schedulable_tasks`, never called by engine
- `get_pending_reminder_tasks` — SQL filter from old scheduler, not used by TaskEngine
- `get_pending_user_reply_task` — from old scheduler, no callers

These dead methods inflate the `AsyncDatabase` API surface (already 845 lines, 60+ methods),
make the API harder to understand, and could mislead future developers into thinking these are
active code paths.

## Findings

- **Source**: architecture-strategist review
- **Location**: `crates/mika-agent/src/async_db.rs:161–224`
- Corresponding `db.rs` methods also exist as dead code
- `get_due_tasks` returns tasks with `next_fire_at <= now` — the engine uses `get_schedulable_tasks`
  (different query semantics)

## Proposed Solutions

### Option A: Remove dead methods (Recommended)
Delete the unused methods from both `async_db.rs` and `db.rs`. Add a comment in `get_schedulable_tasks`
explaining why it's the preferred query over "get_due_tasks".
- **Effort**: Small (dead code — no behavior change)
- **Risk**: Low

### Option B: Mark as #[allow(dead_code)] with explanation
Keep the methods but mark them explicitly with a comment explaining they are reserved for
future use (Phase 4: invoke_orchestrator, user-reply flows).
- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] Dead methods removed (or explicitly documented as intentional future scaffolding)
- [ ] `AsyncDatabase` public API surface reduced
- [ ] All tests pass (dead code removal should not break any tests)

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
