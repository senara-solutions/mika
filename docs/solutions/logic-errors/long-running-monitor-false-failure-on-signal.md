---
title: "Long-running monitor reports false failure on signal-terminated processes"
category: logic-errors
date: 2026-03-19
tags: [exit-code, signal, task-engine, callback, long-running]
module: crates/mika-agent/src/skills/executor.rs
related:
  - docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md
  - docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md
  - docs/solutions/architecture/callback-resume-agent-lifecycle.md
issue: "#207"
---

# Long-running monitor reports false failure on signal-terminated processes

## Problem

When a long-running process (e.g., claude-pilot) completes its work successfully and calls `mika ask --task-id` to deliver results, but then exits via signal (SIGTERM), the background monitor falsely marks the task as `failed` with "Process exited with code -1". This overwrites the already-completed task status, producing misleading failure notifications.

## Root Cause

Two bugs in `spawn_long_running_exec` (executor.rs):

1. **No signal distinction:** `status.code().unwrap_or(-1)` conflates signal termination with a fake exit code `-1`. On Unix, signal-terminated processes return `None` from `status.code()`, but the code collapsed this to `-1` instead of checking `status.signal()`. The synchronous exec handler already handled this correctly.

2. **Race condition in `update_task_failed`:** The function had no status guard — `WHERE id = ? AND agent_id = ?` — so it unconditionally overwrote any status, including `completed` or `delivered`. Compare with `update_task_completed` which correctly guards with `AND status IN ('pending', 'in_progress')`.

## Solution

### 1. Guard `update_task_failed` against terminal states (db.rs)

Added `AND status NOT IN ('completed', 'failed', 'cancelled', 'expired', 'delivered')` to the SQL query, matching the pattern from `update_task_completed`. Changed return type from `Result<()>` to `Result<bool>` for consistency — callers can now distinguish between "task was marked failed" and "task was already in a terminal state."

### 2. Signal-aware exit code formatting (executor.rs, builtin_handlers.rs)

Replaced `status.code().unwrap_or(-1)` with the same `ExitStatusExt::signal()` pattern used by the synchronous exec handler:
- `Some(code)` → `"Exit code: {code}"`
- `None` + signal on Unix → `"Killed by signal: {sig}"`
- `None` + no signal → `"Exit code: unknown"`

### 3. Guard-aware logging

The background monitor now uses `match` on the `Result<bool>` to log at appropriate levels:
- `Ok(true)` → `warn!` (task was marked failed — genuine failure)
- `Ok(false)` → `info!` (task already in terminal state — guard prevented overwrite)
- `Err(e)` → `warn!` (database error)

## Key Insight

When a state-machine transition function (`update_task_failed`) does not guard against the current state, it can silently corrupt already-resolved states. The symmetric pattern — both `update_task_completed` and `update_task_failed` guarding against terminal states — prevents TOCTOU races between the callback completion path and the process exit monitoring path.

## Prevention

- **Symmetric state guards:** When adding state transition functions to a task/job system, always guard against the states you should NOT transition FROM. If `complete` guards, `fail` should too.
- **Never use `unwrap_or(-1)` for exit codes:** On Unix, exit codes are 0-255. A value of -1 is never a real exit code — it means signal termination and should be handled explicitly via `ExitStatusExt::signal()`.
- **Test the race condition:** Add tests that complete a task THEN try to fail it, verifying the completed state is preserved.
