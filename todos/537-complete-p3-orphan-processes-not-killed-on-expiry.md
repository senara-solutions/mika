---
status: complete
priority: p3
issue_id: 537
tags: [code-review, security, skills]
dependencies: []
---

# Orphan Processes Not Killed When Task Expires

## Problem Statement

`spawn_long_running_exec` records the PID via `set_task_process_id`, but when `expire_timed_out_tasks` marks a task as expired, it only updates the DB status — it does not signal the subprocess. The process continues running indefinitely. Combined with the 90-day maximum timeout, zombie processes could accumulate.

**Severity:** P3 — Resource leak in container.

## Findings

- `crates/mika-agent/src/skills/executor.rs:538` — `kill_on_drop(false)` intentional
- `crates/mika-agent/src/task_engine/engine.rs` — `expire_timed_out_tasks` only updates status
- `crates/mika-agent/src/db.rs` — `set_task_process_id` stores PID but no cleanup path

## Proposed Solutions

1. **Send SIGTERM on task expiry**
   - In `expire_timed_out_tasks`, query for expired tasks with process_id set, send SIGTERM
   - Follow up with SIGKILL after grace period
   - Effort: Medium
   - Risk: Low

## Acceptance Criteria

- [ ] Expired tasks with process_id get SIGTERM sent to the subprocess
- [ ] Grace period before SIGKILL
