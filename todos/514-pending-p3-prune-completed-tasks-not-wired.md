---
status: pending
priority: p3
issue_id: "514"
tags: [code-review, performance, database, maintenance]
dependencies: []
---

# `prune_completed_tasks` Not Wired as a Recurring Task — Unbounded DB Growth

## Problem Statement

`db.prune_completed_tasks()` exists but is never called. Completed callback tasks with large `result TEXT` fields (up to 100KB) accumulate in the `tasks` table indefinitely. SQLite stores large TEXT values in overflow pages (~25 overflow pages for a 100KB result), causing measurable I/O amplification at scale.

## Findings

- **Source**: performance-oracle (Scalability Assessment + Recommendation #7)
- **Location**: `crates/mika-agent/src/db.rs` — `prune_completed_tasks` method exists

The `result TEXT` column can hold up to 100KB per row. A 100KB result requires ~25 SQLite overflow pages (4KB default page size). At 1,000 completed callback tasks, this adds up to 25,000 extra page reads for queries touching completed tasks.

The `prune_completed_tasks` method is implemented but never scheduled. It is referenced in tests but not in the task engine startup or recurring task registration.

## Proposed Solutions

### Option A: Schedule as a nightly recurring task (Recommended)

In `task_engine/mod.rs`, add to `ensure_recurring_task` calls at startup:

```rust
ensure_recurring_task(
    db,
    "system:prune-tasks",
    "0 0 3 * * *",  // 3 AM daily
    action_type::RUN_SKILL,
    &serde_json::json!({"trigger": "prune_completed_tasks"}),
    agent_id,
).await?;
```

Or directly in `startup_recovery` / engine initialization as a special system task.

- **Effort**: Small | **Risk**: Low

### Option B: Run in `startup_recovery`

Delete tasks completed more than 30 days ago during startup, not on a schedule.

```rust
db.prune_completed_tasks(30).await?; // delete tasks completed > 30 days ago
```

Simpler, but pruning on every startup adds latency for heavily-used agents.

- **Effort**: Tiny | **Risk**: Low

## Acceptance Criteria

- [ ] Completed tasks older than a configurable retention window (default: 30 days) are automatically deleted
- [ ] Pruning is either scheduled as a nightly recurring task or run at startup
- [ ] `prune_completed_tasks` accepts a `days` parameter to control retention
- [ ] Tests verify that pruning does not delete recent or pending tasks

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
