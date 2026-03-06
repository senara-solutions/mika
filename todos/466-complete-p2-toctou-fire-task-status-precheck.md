---
status: pending
priority: p2
issue_id: "466"
tags: [code-review, correctness, task-engine, concurrency]
dependencies: []
---

# 466 · TOCTOU race in `fire_task` — cancel between status check and `update_task_status`

## Problem Statement

`fire_task` reads task status from DB to pre-check it is `pending`, then
separately updates it to `in_progress`. Between these two operations, a
concurrent `cancel_task` call (e.g., from the `cancel_reminder` tool
running in the agent loop) can set the task to `cancelled`. The subsequent
`update_task_status(..., "in_progress")` then unconditionally overwrites
`cancelled` → `in_progress`, executing a task the user just cancelled.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/engine.rs:294–309`
- `cancel_task` uses `WHERE status NOT IN ('completed','failed','cancelled','expired')` guard on its own UPDATE — so the cancel writes first if it wins the race. But `update_task_status` has no such guard and overwrites unconditionally.
- The `TaskEngine` mutex is held during `fire_task`, but the cancel arrives via `AsyncDatabase` which has its own channel — the mutex does not protect against out-of-engine DB writes.

## Proposed Solutions

### Option A — Atomic conditional UPDATE (recommended)
Replace `update_task_status(..., "in_progress")` with a conditional:
```sql
UPDATE tasks SET status = 'in_progress', updated_at = unixepoch()
WHERE id = ?1 AND status IN ('pending', 'recurring_active')
```
Check `rows_affected > 0`; if 0, the task was cancelled/completed — skip dispatch.

**Pros:** Eliminates the race entirely. Simple.
**Effort:** Small | **Risk:** Low

### Option B — Mutex held through the cancel tool too
Require the cancel tool to acquire the engine lock before writing.
**Cons:** Engine lock is not accessible from tool context. Not practical.

## Recommended Action

Option A. Add a `try_claim_task(id) -> Result<bool>` DB method.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`, `crates/mika-agent/src/task_engine/engine.rs`

## Acceptance Criteria

- [ ] `try_claim_task` method added that uses conditional `WHERE status IN ('pending','recurring_active')`
- [ ] `fire_task` checks the return value and skips dispatch if `false`
- [ ] Existing `get_task` pre-check in `fire_task` can be removed once claim is atomic

## Work Log

- 2026-03-06: Identified by security review agent (COR-4)
