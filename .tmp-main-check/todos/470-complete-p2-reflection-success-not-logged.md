---
status: pending
priority: p2
issue_id: "470"
tags: [code-review, correctness, task-engine, audit]
dependencies: []
---

# 470 · Reflection audit log is asymmetric — successful runs not recorded

## Problem Statement

`dispatch_reflection` calls `record_reflection_run("failed", ...)` on error
but makes no `record_reflection_run` call on success. The `reflection_runs`
table is queried by `last_reflection_run_today` to gate whether reflection
should run again that day. An unrecorded successful run means reflection can
fire again the same day on the next tick after a process restart, running
twice in one day.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/dispatcher.rs:262–268`
- `last_reflection_run_today` at line ~201 queries the `reflection_runs` table — if no success row exists, reflection will re-trigger
- The old `scheduler.rs` (now deleted) did record successful runs; this was lost in the migration

## Proposed Solutions

### Option A — Add success-path `record_reflection_run` call (recommended)
After `run_silent_agent` returns `Ok`:
```rust
let _ = self.db.record_reflection_run("completed", 0, None).await;
```

**Effort:** Trivial | **Risk:** Low

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/dispatcher.rs`

## Acceptance Criteria

- [ ] `record_reflection_run("completed", ...)` called after successful `run_silent_agent`
- [ ] Test: run reflection dispatch, assert `reflection_runs` table has one row with `status='completed'`
- [ ] Test: `last_reflection_run_today` returns `true` after a successful run

## Work Log

- 2026-03-06: Identified by code quality review agent (QUAL-4)
