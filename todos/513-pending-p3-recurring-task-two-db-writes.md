---
status: pending
priority: p3
issue_id: "513"
tags: [code-review, performance, database]
dependencies: []
---

# Two Separate DB Writes for Recurring Task Re-enqueue — Should Be One

## Problem Statement

`engine.rs` sends two separate `UPDATE` statements for every recurring task completion: one for `next_fire_at` and one for `status`. Each goes through the `mpsc` channel to the DB thread and back via `oneshot`. This is two round-trips where one would suffice.

## Findings

- **Source**: performance-oracle (Optimization #4)
- **Location**: `crates/mika-agent/src/task_engine/engine.rs:344-353`

```rust
if let Err(e) = db.update_task_next_fire_at(&task_id, next).await {
    warn!(...);
}
if let Err(e) = db.update_task_status(&task_id, task_status::RECURRING_ACTIVE).await {
    warn!(...);
}
```

Two sequential async DB calls where one combined `UPDATE` would suffice. At current heartbeat frequency (hourly) this is negligible. For high-frequency recurring tasks it adds unnecessary channel overhead.

## Proposed Solutions

### Option A: Add `update_task_rescheduled` method (Recommended)

```rust
// In db.rs:
pub fn update_task_rescheduled(&self, id: &str, next_fire_at: i64) -> Result<()> {
    self.conn.execute(
        "UPDATE tasks SET next_fire_at = ?1, status = 'recurring_active', updated_at = unixepoch() WHERE id = ?2",
        params![next_fire_at, id],
    )?;
    Ok(())
}
```

Replace the two calls in `engine.rs` with the single combined call.

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] `update_task_rescheduled` method exists in `db.rs` combining `next_fire_at` + `status` in one `UPDATE`
- [ ] `engine.rs` uses the combined method for recurring task re-enqueue
- [ ] `AsyncDatabase` wrapper updated to expose the new method
- [ ] Existing tests pass

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
