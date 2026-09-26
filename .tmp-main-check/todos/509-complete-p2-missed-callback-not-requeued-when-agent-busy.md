---
status: complete
priority: p2
issue_id: "509"
tags: [code-review, reliability, performance, agent-native]
dependencies: []
---

# Missed Callback Silently Dropped When Agent Is Busy (`try_lock` Failure)

## Problem Statement

When `dispatch_resume_agent` or `dispatch_skill_by_name` calls `try_lock` on the agent lock and fails (agent is currently processing a turn), the function returns `Ok(())` silently. The callback result is permanently lost — there is no retry or re-enqueue mechanism.

## Findings

- **Source**: performance-oracle (recommendation)
- **Location**: `crates/mika-agent/src/task_engine/dispatcher.rs`

Both dispatch functions follow this pattern:
```rust
let lock = match agent_state.agent_lock.try_lock() {
    Ok(g) => g,
    Err(_) => {
        debug!(task_id, "agent busy, skipping dispatch");
        return Ok(());
    }
};
```

`Ok(())` means "I successfully did nothing" — the task is now in `completed` state (for callbacks) or remains `in_progress` (for skill tasks, which use a different flow), but the agent was never run.

For callback tasks: after `update_task_completed` runs in the HTTP handler, `dispatch_completed_callback` is spawned. If the spawned task runs while the agent is busy, the callback result is permanently lost. The task is `completed` with a result, but the agent never saw it.

For skill tasks: same pattern — the task fires from the tick loop, hits a busy agent, and the scheduled work is skipped.

## Proposed Solutions

### Option A: Re-enqueue with backoff when agent is busy (Recommended)

```rust
Err(_) => {
    warn!(task_id, "agent busy, re-enqueuing callback in 30s");
    let retry_at = Utc::now().timestamp() + 30;
    // Update task next_fire_at and set status back to pending
    // This puts it back in the engine's scheduler queue
    db.reschedule_task(task_id, retry_at).await?;
    return Ok(());
}
```

For callback tasks specifically: introduce a `retry_count` or use `next_fire_at` to schedule a re-check. Cap retries at 3 to avoid infinite loops when an agent is permanently stuck.

- **Effort**: Medium | **Risk**: Moderate (requires re-enqueue path in dispatcher)

### Option B: Brief backoff with retry in the spawned task

```rust
// In the tokio::spawn closure in handle_task_complete:
tokio::time::sleep(Duration::from_millis(500)).await;
// Retry the lock up to 3 times before giving up
```

Simpler but only helps for brief lock contention, not for long-running agent turns.

- **Effort**: Small | **Risk**: Low

### Option C: Document and accept as known limitation

Add a `warn!` noting the callback was dropped, expose it in logs. Accept that callbacks during active agent turns are lost.

- **Effort**: Tiny | **Risk**: None (no behavioral change)

## Acceptance Criteria

- [ ] When `try_lock` fails in a callback dispatch, the result is not silently dropped
- [ ] Callback tasks are re-enqueued for retry with a backoff when the agent is busy
- [ ] Retry count is capped to prevent infinite retry loops
- [ ] A `warn!` log is emitted when a callback is dropped or re-enqueued

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
