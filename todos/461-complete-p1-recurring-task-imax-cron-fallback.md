---
status: complete
priority: p1
issue_id: "461"
tags: [code-review, correctness, task-engine, cron]
dependencies: []
---

# 461 · Recurring task uses `i64::MAX` on cron failure — silently disabled forever

## Problem Statement

When a recurring task's cron expression is missing or invalid, the
`fire_task` spawned closure falls back to `i64::MAX` as the next fire
timestamp. This sentinel is written to the DB and re-enqueued with
`next_fire_at = i64::MAX` (year ~292 billion). The task stays in
`recurring_active` state, accumulates in every DB scan, but never fires
again. There is no error log at `warn` level at the `unwrap_or` call site,
making this a silent permanent failure with no recovery path.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/engine.rs:326–331`
- The `ok()` call before `unwrap_or` silently discards the cron parse error
- `enqueue_queued_task` does log a `warn!` when it picks this task up again on periodic scan, but the task is never removed from the DB
- Builtin tasks use hardcoded cron strings so the immediate risk is low, but any future tool-created recurring task with a bad expression would hit this

## Proposed Solutions

### Option A — Explicitly mark task failed on cron error (recommended)
```rust
let next = match cron_expr.as_deref()
    .ok_or_else(|| anyhow!("missing cron_expr"))
    .and_then(|e| next_fire_from_cron(e, chrono::Utc::now().timestamp()))
{
    Ok(ts) => ts,
    Err(e) => {
        warn!(task_id = %task_id, error = %e, "cannot reschedule recurring task, marking failed");
        let _ = db.update_task_failed(&task_id, &e.to_string()).await;
        return;
    }
};
```

**Pros:** Observable failure, task removed from recurring cycle, recoverable.
**Effort:** Small | **Risk:** Low

### Option B — Retry with exponential backoff
On cron error, schedule retry in 60 seconds rather than `i64::MAX`.

**Pros:** Transient failures self-heal.
**Cons:** Bad cron expressions loop forever.
**Effort:** Small | **Risk:** Low (but masks bad data)

## Recommended Action

Option A. Bad cron expressions should fail loudly, not retry silently.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/engine.rs`
- Depends on todo 459 (add `update_task_failed`)

## Acceptance Criteria

- [ ] `unwrap_or(i64::MAX)` replaced with explicit error branch
- [ ] Failed cron computation logs `warn!` with task_id and error
- [ ] Task is marked `failed` in DB, not left as `recurring_active`
- [ ] Test: create recurring task with invalid cron, tick, assert status == 'failed'

## Work Log

- 2026-03-06: Identified by security (COR-3) and architecture (ARCH-4) review agents
