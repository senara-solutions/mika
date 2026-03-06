---
status: complete
priority: p1
issue_id: "498"
tags: [code-review, architecture, correctness, database]
dependencies: []
---

# TOCTOU Race in `update_task_completed` Allows Duplicate Agent Dispatch

## Problem Statement

`handle_task_complete` performs three separate non-atomic operations: read task status, update to completed, spawn agent dispatch. Two concurrent POST requests to `/tasks/{id}/complete` can both pass the status check before either commits, causing `update_task_completed` to fire twice and the agent to be dispatched twice for the same callback result.

## Findings

- **Source**: architecture-strategist (F-1 Critical), performance-oracle (F-1)
- **Location**: `crates/mika-agent/src/db.rs:811`, `crates/mika-agent/src/server/handlers.rs:332-406`

The `update_task_completed` SQL at db.rs:811:
```sql
UPDATE tasks SET status = 'completed', result = ?1, completed_at = unixepoch()
WHERE id = ?2
```
Has no guard on `AND status IN ('pending', 'in_progress')`. The handler checks status before calling this update (lines 363-370), but the check and update are separate round-trips with no transaction or atomic guard. Two concurrent callers can both pass the check window.

Impact: Agent runs twice for the same callback result. Second run receives an already-completed task context and may produce duplicate `send_message` calls or corrupt memory state.

## Proposed Solutions

### Option A: Add status guard to SQL (Recommended)

```sql
UPDATE tasks
SET status = 'completed', result = ?1, completed_at = unixepoch()
WHERE id = ?2 AND status IN ('pending', 'in_progress')
```

Then in `update_task_completed`, return the affected row count (rusqlite `execute()` returns `usize`). In the handler: if count == 0, return 409 Conflict. This collapses check + update into one atomic DB round-trip.

Add a new `AsyncDatabase` wrapper variant that returns the row count, or change `update_task_completed` signature to `-> Result<bool>` (true = updated, false = already completed/cancelled).

- **Effort**: Small | **Risk**: None

### Option B: Wrap in SQLite transaction

Use a `BEGIN IMMEDIATE` transaction around the read + update. More complex, requires exposing transaction control in `AsyncDatabase`.

- **Effort**: Medium | **Risk**: Moderate (changes AsyncDatabase threading model)

## Acceptance Criteria

- [ ] `update_task_completed` SQL includes `AND status IN ('pending', 'in_progress')` guard
- [ ] Handler returns 409 Conflict when the affected row count is 0
- [ ] Concurrent duplicate POST to `/tasks/{id}/complete` results in exactly one agent dispatch (test or reasoning)
- [ ] Existing server tests pass

## Work Log

- 2026-03-06: Identified by architecture-strategist and performance-oracle reviews of feat/unified-task-engine
