---
title: "fix: Improve long-running process exit code handling"
type: fix
status: active
date: 2026-03-19
---

# fix: Improve long-running process exit code handling

## Overview

When a long-running process (e.g., claude-pilot) completes its work successfully and calls mika ask --task-id to report results, but then exits via signal (SIGTERM), the background monitor falsely marks the task as failed with "Process exited with code -1". This creates misleading failure notifications that require manual verification to distinguish from real errors.

## Problem Statement

Two bugs in spawn_long_running_exec (executor.rs, lines 667-690):

1. No signal distinction: status.code().unwrap_or(-1) conflates signal termination with a fake exit code -1. The synchronous exec handler (lines 393-408) properly distinguishes signals via ExitStatusExt::signal(), but the long-running monitor does not.

2. Race condition: update_task_failed overwrites completed tasks. update_task_failed has no status guard (WHERE id = ? AND agent_id = ?), so it unconditionally overwrites any status including completed or delivered. Compare with update_task_completed which correctly guards with AND status IN ('pending', 'in_progress').

A secondary inconsistency exists in builtin_handlers.rs line 368, which also uses unwrap_or(-1) for signal-terminated processes.

## Proposed Solution

Guard-only fix with improved message formatting. No new task status -- adding a killed status would require a schema v13 migration (full table rebuild), updates to 6+ SQL queries with terminal-state lists, dashboard UI changes, and CHECK constraint modification. The guard-only approach achieves all acceptance criteria without that cost.

### Changes

1. Guard update_task_failed against terminal states (db.rs) - add status guard, change return type from Result to Result bool
2. Add signal distinction in long-running monitor (executor.rs) - use ExitStatusExt::signal() on Unix
3. Log when guard prevents overwrite (executor.rs) - check return value, log at info level
4. Fix signal distinction in builtin_handlers.rs - same signal-aware pattern
5. Guard the wait-failed path (executor.rs) - lines 657-663

## Acceptance Criteria

- Root cause identified: status.code().unwrap_or(-1) + missing status guard on update_task_failed
- update_task_failed guards against overwriting terminal states (returns Result bool)
- Long-running monitor distinguishes signal termination from non-zero exit codes
- builtin_handlers.rs uses signal-aware exit code formatting
- wait-failed path in spawn_long_running_exec also guards against overwriting
- All callers of update_task_failed updated for Result bool return type
- Tests: race condition (complete then fail stays completed), signal formatting

## Files to Modify

- crates/mika-agent/src/db.rs - Add status guard + change return type on update_task_failed
- crates/mika-agent/src/async_db.rs - Update update_task_failed wrapper return type
- crates/mika-agent/src/skills/executor.rs - Signal distinction + guard-aware logging
- crates/mika-agent/src/skills/builtin_handlers.rs - Signal-aware exit code formatting
- crates/mika-agent/src/task_engine/engine.rs - Update startup_recovery caller

## Sources and References

- Related issue: #207
- Pattern reference: update_task_completed guard at db.rs line 2228-2231
- Sync handler signal distinction: executor.rs lines 393-408
