---
status: complete
priority: p1
issue_id: "459"
tags: [code-review, correctness, task-engine, database]
dependencies: []
---

# 459 · Failed task dispatch writes `status='completed'` instead of `'failed'`

## Problem Statement

When the `TaskDispatcher::dispatch()` returns an `Err`, the engine calls
`db.update_task_completed(&task_id, Some(&e.to_string()))`. But
`update_task_completed` unconditionally sets `status = 'completed'`. Every
failed task is permanently recorded as successful in the DB. Operators and
tools querying `status = 'failed'` will see zero rows even when every task
dispatch is erroring. `startup_recovery` will also not pick up these tasks for
retry since they appear completed.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/engine.rs:358–361`
- **DB method:** `crates/mika-agent/src/db.rs:769–777` — `update_task_completed` sets `status = 'completed'` unconditionally
- The `tasks` table `status` CHECK constraint already includes `'failed'` — it is just never written on the error path
- `startup_recovery` correctly uses `status = 'failed'` for orphaned in_progress tasks, proving the intent is there

## Proposed Solutions

### Option A — Add `update_task_failed` method (recommended)
Add `Database::update_task_failed(id, error: &str)` setting `status='failed'`, `result=error`, `completed_at=unixepoch()`. Call it in the `Err` arm of the spawned fire closure instead of `update_task_completed`.

**Pros:** Semantically correct. No change to `update_task_completed` callers.
**Cons:** One new DB method.
**Effort:** Small | **Risk:** Low

### Option B — Add a `failed` parameter to `update_task_completed`
Accept `success: bool` and branch on it in the SQL.

**Pros:** Fewer methods.
**Cons:** Confusing API (`update_task_completed(id, false, error)`).
**Effort:** Small | **Risk:** Low

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`, `crates/mika-agent/src/task_engine/engine.rs`
- **Schema:** `tasks.status CHECK (status IN ('pending','in_progress','completed','failed','cancelled','expired','recurring_active'))`

## Acceptance Criteria

- [ ] `update_task_failed` method added to `Database` and `AsyncDatabase`
- [ ] `Err` arm in `fire_task` spawned closure calls `update_task_failed`
- [ ] Test: create a task, force dispatch failure (mock or bad action_type), assert `status == 'failed'` in DB

## Work Log

- 2026-03-06: Identified by security/correctness and architecture review agents (COR-1, ARCH-2)
