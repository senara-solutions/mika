---
title: "Failed callback tasks silently dropped — agent never notified"
category: logic-errors
date: 2026-03-18
tags: [callback, task-engine, silent-failure, status-filter, polling]
issue: 203
---

# Failed Callback Tasks Silently Dropped

## Problem

When a long-running exec handler exits with a non-zero code, the background monitor marks the task as `status = 'failed'` in SQLite. However, the agent is **never notified** because all callback delivery paths — TUI polling, server-mode dispatch, and the `mark_task_delivered` atomic claim — hardcode `status = 'completed'` in their SQL WHERE clauses. The agent waits indefinitely for a callback that will never arrive.

**Symptom:** `mika tasks list` shows `status: failed` with an error message, but the agent (and user) receive no notification. The TUI appears stuck waiting.

## Root Cause

Four code locations filtered exclusively on `status = 'completed'`:

1. `db.rs` — `get_undelivered_callback_tasks()` SQL query
2. `db.rs` — `get_undelivered_callback_tasks_for_session()` SQL query
3. `db.rs` — `mark_task_delivered()` SQL UPDATE filter
4. `db.rs` — `idx_tasks_callback_delivery` partial index predicate

Additionally:
- The TUI error-recovery path hardcoded reset to `'completed'`, which would corrupt a failed task's status on agent error
- `format_callback_framing()` always said "A background task has completed" even for failures
- Server mode had no mechanism to deliver failed callbacks (no external trigger fires for background monitor failures)

## Solution

### 1. Widen DB status filters

Change all four SQL locations from `status = 'completed'` to `status IN ('completed', 'failed')`:

```sql
-- Queries: get_undelivered_callback_tasks, get_undelivered_callback_tasks_for_session
AND status IN ('completed', 'failed')

-- Atomic claim: mark_task_delivered
WHERE id = ?1 AND status IN ('completed', 'failed')

-- Partial index
CREATE INDEX idx_tasks_callback_delivery ON tasks(agent_id, completed_at)
WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');
```

### 2. TUI: distinguish success from failure

- Show `[label] failed` system message for failed tasks (instead of always "completed")
- Failed tasks with empty results get fallback: `FAILED_TASK_FALLBACK` constant
- Thread `original_status` through `AgentRequest::CallbackResult` so error-recovery resets to the correct status

### 3. Agent framing: differentiate completed vs failed

`format_callback_framing(label, task_id, result, failed: bool)` — when `failed=true`, uses "A background task has FAILED" preamble. The `<callback_result trust="untrusted">` wrapper stays the same.

### 4. Server mode: periodic scan for undelivered callbacks

Added `dispatch_undelivered_callbacks()` to the engine's periodic tick (every 60 ticks). Scans for `status IN ('completed', 'failed')` callback tasks and dispatches them via `dispatch_resume_agent`. This covers failed callbacks from the background monitor that have no external trigger.

### 5. Task status state machine

```
pending → in_progress → completed → delivered
                      → failed    → delivered
```

Both `completed` and `failed` are now "deliverable" terminal states.

## Key Insight

When adding a new terminal status to a task state machine, audit **every** query that filters on the existing terminal status. The bug was that `'failed'` was a valid terminal state (set by the background monitor) but was invisible to the delivery pipeline because all queries were written when `'completed'` was the only deliverable status. The partial index predicate was especially easy to miss.

## Prevention

- When introducing a new status value, grep for all SQL queries that reference adjacent statuses in the same lifecycle stage. A status filter audit checklist:
  1. SELECT queries (polling/scanning)
  2. UPDATE queries (atomic claims/transitions)
  3. Partial index predicates
  4. Error-recovery paths that reset status
  5. Display/framing code that assumes a specific status

## Related

- `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md` — original callback delivery architecture
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — loop prevention invariants (maintained for failed callbacks)
- `docs/solutions/architecture/callback-resume-agent-lifecycle.md` — full callback lifecycle
