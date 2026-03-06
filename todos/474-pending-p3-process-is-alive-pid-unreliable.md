---
status: pending
priority: p3
issue_id: "474"
tags: [code-review, correctness, task-engine, startup]
dependencies: []
---

# 474 · `process_is_alive` PID check is unreliable — `process_id` is never set

## Problem Statement

`startup_recovery` calls `process_is_alive(pid)` to skip marking a task
`failed` if the original process is still running. Two problems:

1. **`process_id` is never written:** No callsite calls `set_task_process_id`
   before dispatch. Every task has `process_id = NULL`, so `process_is_alive`
   is dead code in practice.
2. **PID reuse across container restarts:** `/proc/{pid}` is not reliable
   across container restarts — a new container process can reuse the same PID,
   causing `process_is_alive` to return `true` for a dead process and skip
   recovery.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/engine.rs:372–382`
- No callsite to `set_task_process_id` exists in the codebase (`rg set_task_process_id` returns nothing)

## Proposed Solutions

### Option A — Remove PID check, always recover orphaned tasks (recommended)
Since `process_id` is never set, the check always evaluates to `false`
(pid is 0/NULL). Simplify `startup_recovery` to unconditionally mark all
`in_progress` tasks as `failed` on restart.

**Effort:** Trivial | **Risk:** Low

### Option B — Replace with instance-ID approach
Generate a UUID at startup, store it with each task at claim time, compare on recovery.
**Effort:** Medium | **Risk:** Low (better design)

## Recommended Action

Option A now, Option B if per-process task survival tracking is ever needed.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/engine.rs`, `crates/mika-agent/src/db.rs`

## Acceptance Criteria

- [ ] `process_is_alive` function removed or guarded behind a `process_id IS NOT NULL` check
- [ ] `startup_recovery` unconditionally marks orphaned `in_progress` tasks as `failed`

## Work Log

- 2026-03-06: Identified by architecture (ARCH-12) and security (SEC-1) review agents
