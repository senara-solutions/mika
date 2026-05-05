---
module: task-engine
tags: [callback, watchdog, process-liveness, dispatch-queue, self-dev]
problem_type: reliability
category: task-engine
---

# Stale Callback Watchdog: Process Liveness Detection (#959)

## Problem

When a `run_claude_pilot` subprocess crashes without delivering its callback, the callback task remains `in_progress` indefinitely. The global single-session guard (`has_active_callback_tasks_excluding`) blocks all subsequent dispatches with `"global_dispatch_active"` error. The existing `timeout_at` mechanism (6h for claude-pilot) is too slow — a subprocess that crashes at minute 1 blocks the queue for 5h59m until manual intervention.

### Incident evidence

On 2026-05-02, callback `60be6094` became stale. Three distinct ticket dispatches (mika#928, #927, #952) blocked simultaneously. The autonomous dev loop halted for ~30 minutes until manual operator intervention.

## Solution

Added a **process liveness watchdog** to the engine's periodic tick loop that detects dead subprocesses via PID + `/proc/<pid>/stat` process start time checks.

### Key design decisions

1. **PID liveness, not timeout.** `kill(pid, 0)` + `/proc/<pid>/stat` field 22 (starttime) comparison detects death in ~60s (one engine tick) vs 6 hours for `timeout_at`.

2. **PID reuse detection.** Process start time (clock ticks since boot) stored at spawn in callback task metadata. If a process exists at the PID but has a different start time, it's a reused PID — treat as dead.

3. **Grace period.** 120s (configurable via `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`) after first detection of subprocess death. Allows for in-flight callback delivery that may be in transit when the subprocess exits cleanly. Uses `first_dead_at` metadata field to track when death was first detected.

4. **`failed` not `expired`.** The task crashed, it didn't time out. Uses `update_task_failed` with `error_reason = "subprocess_exited_without_delivery"` for audit clarity. The existing orphaned parent reaper (#871) already handles `failed` callbacks correctly.

5. **Race guard.** Before marking `failed`, re-reads task status from DB. If the task transitioned during the grace period (callback delivered), skips.

6. **Platform: Linux only.** Uses `/proc/<pid>/stat` — consistent with existing `process_kill.rs` and container-isolated deployment model.

### Implementation

| Component | File | Purpose |
|-----------|------|---------|
| Process liveness | `task_engine/process_liveness.rs` | `read_process_start_time()`, `is_same_process_alive()` |
| Engine watchdog | `task_engine/engine.rs` | `check_callback_process_liveness()` in tick loop |
| Spawn recording | `skills/executor.rs` | Stores `process_start_time` in callback task metadata |
| DB helpers | `db.rs` | `get_active_callback_tasks_with_pid()`, `set_task_metadata_field()`, `remove_task_metadata_field()` |
| Config | `config.rs` | `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120) |

### Interaction with existing mechanisms

| Mechanism | Relationship |
|-----------|-------------|
| `expire_timed_out_tasks` | Panic-fallback (6h). Watchdog is primary detection (~3min). |
| `kill_orphan_processes` | Complementary. Fires on already-expired tasks. Watchdog detects death *before* expiry. |
| `reap_orphaned_parent_tasks` | Downstream consumer. After watchdog marks callback `failed`, reaper handles the parent. |

### Gotcha: metadata field cleanup

When clearing `first_dead_at` (process came back alive), use `remove_task_metadata_field` (SQLite `json_remove`) not `set_task_metadata_field(..., "null")`. The latter stores the string `"null"`, which is parseable as `Some("null")` on subsequent reads, breaking the grace period detection.

## Verification

After deploying, confirm with:

```bash
# Watchdog not firing on healthy subprocesses (no false positives)
grep callback_watchdog_detected_process_death server.log | wc -l
# Should be 0 on healthy restarts

# Watchdog detection on subprocess crash
# Kill a claude-pilot subprocess mid-execution, wait ~3 min, check:
grep callback_watchdog_detected_process_death server.log
# Should show the killed callback_id and pid
```
