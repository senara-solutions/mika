---
status: pending
priority: p1
issue_id: "460"
tags: [code-review, correctness, task-engine]
dependencies: []
---

# 460 · `inject_context` task lifecycle is silently broken

## Problem Statement

`inject_context` tasks are entirely non-functional. Two separate bugs combine
to ensure the injected payload is never consumed by the agent loop:

1. **Wrong field check in `fire_task`:** The engine special-cases
   `inject_context` tasks to avoid marking them `completed` post-dispatch, but
   checks `trigger_type != "inject_context"` — while the tasks are created with
   `action_type = "inject_context"` and `trigger_type = "time"`. So the guard
   never fires, and the task is immediately marked `completed` by the success
   branch.

2. **Status filter mismatch:** `get_inject_context_tasks` queries
   `status = 'pending'`, but `fire_task` unconditionally sets the task to
   `in_progress` before dispatch. Even if bug 1 were fixed, the agent loop
   would never find the task.

## Findings

- **Bug 1 location:** `crates/mika-agent/src/task_engine/engine.rs:349` — checks `trigger_type`, should check `action_type`
- **Bug 2 location:** `crates/mika-agent/src/db.rs:854–866` — queries `status = 'pending'`, but task is `in_progress` by the time the agent loop runs
- `QueuedTask` struct only carries `trigger_type`, not `action_type` — the engine cannot check `action_type` without extending the struct or adding a DB re-fetch

## Proposed Solutions

### Option A — Fix both fields (recommended)
1. Add `action_type: String` to `QueuedTask` and populate it in `enqueue_queued_task`.
2. Change the guard in `fire_task` to `queued.action_type != "inject_context"`.
3. Change `get_inject_context_tasks` to query `status IN ('pending', 'in_progress')`.
4. After agent loop consumes an inject_context task, call `update_task_completed`.

**Pros:** Semantically correct end-to-end.
**Effort:** Medium | **Risk:** Low

### Option B — Don't set inject_context tasks to `in_progress` at fire time
Skip the `update_task_status("in_progress")` call for `inject_context` action types. Agent loop consumes them while still `pending`.

**Pros:** Simpler — no struct change needed.
**Cons:** Orphaned inject_context tasks stay `pending` forever if agent loop crashes before consuming.
**Effort:** Small | **Risk:** Low

## Recommended Action

Option A for correctness.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/engine.rs`, `crates/mika-agent/src/task_engine/queue.rs`, `crates/mika-agent/src/db.rs`

## Acceptance Criteria

- [ ] `QueuedTask` carries `action_type`
- [ ] Guard in `fire_task` checks `action_type`, not `trigger_type`
- [ ] `get_inject_context_tasks` queries `status IN ('pending','in_progress')`
- [ ] Agent loop marks inject_context task `completed` after consuming it
- [ ] Integration test: create inject_context task, tick, run agent loop, assert task consumed

## Work Log

- 2026-03-06: Identified by security (COR-2, COR-7) and architecture (ARCH-9) review agents
