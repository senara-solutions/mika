---
status: pending
priority: p3
issue_id: "493"
tags: [code-review, performance, task-engine]
dependencies: []
---

# fire_task Makes Two Sequential DB Round-Trips Before spawning Dispatch Task

## Problem Statement

`fire_task` in `engine.rs` calls `db.try_claim_task(&task_id).await` and then
`db.set_task_fired(&task_id).await` as two separate SQL UPDATE statements sent over the
AsyncDatabase mpsc channel. The engine's `Mutex` is held for the duration of both awaits. With
`MAX_PER_TICK = 10`, up to 20 channel operations are queued per tick while the engine lock is
held. Merging the two operations into a single SQL statement would halve the DB channel pressure
per fired task.

## Findings

- **Source**: performance-oracle review
- **Location**: `crates/mika-agent/src/task_engine/engine.rs` (fire_task function)
- At current task volumes (2–10 tasks), impact is unmeasurable — optimization opportunity
- `try_claim_task` + `set_task_fired` can be combined into one UPDATE with `RETURNING` or
  merged into a single `claim_and_fire_task` DB method

## Proposed Solutions

### Option A: Add claim_and_fire_task method to Database (Recommended)
```rust
pub fn claim_and_fire_task(&self, id: &str) -> Result<bool> {
    let n = self.conn.execute(
        "UPDATE tasks SET status = 'in_progress', fired_at = unixepoch(), updated_at = unixepoch()
         WHERE id = ?1 AND status IN ('pending', 'recurring_active')",
        params![id],
    )?;
    Ok(n > 0)
}
```
Reduces two DB round-trips to one per fired task.
- **Effort**: Small | **Risk**: Low

### Option B: Keep current design (acceptable at current scale)
At current volumes (2–10 tasks), the overhead is negligible relative to LLM latency.
Document as a future optimization.
- **Effort**: None | **Risk**: None

## Acceptance Criteria

- [ ] `fire_task` uses a single DB call to claim and mark a task as fired (if Option A chosen)
- [ ] Behavior is equivalent to the two-call approach
- [ ] Tests pass

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
