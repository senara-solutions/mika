---
status: pending
priority: p2
issue_id: "504"
tags: [code-review, architecture, reliability, recovery]
dependencies: []
---

# Callback Tasks Skip `in_progress` State — No Recovery Path if Process Killed Mid-Dispatch

## Problem Statement

Callback tasks transition directly from `pending` to `completed`, skipping `in_progress` entirely. If the process is killed between `update_task_completed` and the spawned `dispatch_completed_callback` executing, the task is permanently `completed` but the agent was never resumed. `startup_recovery` cannot detect or recover this — it only handles `in_progress` orphans.

## Findings

- **Source**: architecture-strategist (F-5 Medium)
- **Location**: `crates/mika-agent/src/server/handlers.rs:373-406`

Current lifecycle: `pending → completed` (no `in_progress` step).

`startup_recovery` step 2 marks orphaned `in_progress` tasks as `failed`. This correctly handles time/recurring tasks that were mid-execution at restart. But callback tasks never enter `in_progress`, so if the process is killed after `update_task_completed` writes but before `dispatch_completed_callback` runs (the tokio::spawn future), the task is `completed` but the agent was never resumed. There is no recovery path.

The window is narrow (between DB commit and tokio executor running the spawned future), but it is a real "lost wakeup" scenario with no observability.

## Proposed Solutions

### Option A: Set `in_progress` before spawn, let dispatcher set final state (Recommended)

In `handle_task_complete`:
```rust
// Atomically transition: pending → in_progress (set status + result)
db.begin_task_callback(task_id, &req.result).await?; // sets status='in_progress', result=req.result

let dispatcher = { engine.lock().await.dispatcher() };
tokio::spawn(async move {
    match dispatcher.dispatch_completed_callback(&completed_task).await {
        Ok(_) => db.update_task_status(id, task_status::COMPLETED).await,
        Err(e) => {
            warn!("callback dispatch failed: {e}");
            db.update_task_status(id, task_status::FAILED).await
        }
    }
});
```

`startup_recovery` already marks orphaned `in_progress` tasks as `failed`. Callback-in-progress tasks that die mid-dispatch are then correctly marked `failed` on next startup, giving visibility and a retry path.

- **Effort**: Medium | **Risk**: Moderate (changes handler + db.rs + dispatcher)

### Option B: Document the lost wakeup window, add warning log

Add a `warn!` after the spawn noting that if the process dies before the future executes, the task will appear completed but the agent was not resumed.

- **Effort**: Tiny | **Risk**: None (documents limitation)

### Option C: Use `tokio::spawn` and set completed in dispatcher (same as A without the handler change)

Have `dispatch_completed_callback` call `db.update_task_status(COMPLETED)` on success and `db.update_task_status(FAILED)` on error. Only requires adding a `db` ref to the dispatcher. The handler still sets `in_progress` atomically before spawning.

- **Effort**: Medium | **Risk**: Low

## Acceptance Criteria

- [ ] Callback tasks transition through `pending → in_progress → completed/failed`
- [ ] `startup_recovery` correctly marks callback-in-progress tasks as `failed` on restart
- [ ] Handler returns 200 after `in_progress` transition (not after full agent dispatch)
- [ ] `dispatch_completed_callback` sets task to `completed` on success or `failed` on error

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
