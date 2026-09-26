# Plan: Stale Callback Watchdog for Self-Dev Dispatch (#959)

type: fix
ticket: mika issue#959
branch: fix/959/self-dev-stale-long-running-callback
date: 2026-05-05

## Problem

When a `run_claude_pilot` subprocess crashes without delivering its callback, the callback task remains in `pending`/`in_progress` indefinitely. The global single-session guard (`has_active_callback_tasks_excluding`) then blocks all subsequent dispatches with `"global_dispatch_active"` error. The existing `timeout_at` mechanism (default 6h for claude-pilot) is too slow to detect a dead subprocess — the loop halts for hours until manual intervention.

## Root Cause

The existing timeout mechanism (`expire_timed_out_tasks` in `engine.rs`) relies on `timeout_at` which is set to `estimated_duration × 3` (clamped 10min–90days). For `run_claude_pilot` with default 7200s estimated duration, this yields a 6-hour timeout. A subprocess that crashes at minute 1 blocks the queue for 5h59m.

The separate `kill_orphan_processes` mechanism only fires on already-expired tasks. There's no liveness check that detects "subprocess PID is gone but callback task is still active."

## Solution: Process Liveness Watchdog

Add a **process liveness check** to the engine's periodic tick loop that detects when a callback task's subprocess has exited (PID no longer running) but the callback hasn't been delivered. When detected, mark the callback `failed` with reason `subprocess_exited_without_delivery`.

### Design Decisions

1. **Detect by PID liveness, not by timeout.** The subprocess PID is stored in `tasks.process_id`. If `kill(pid, 0)` fails with ESRCH (no such process), the subprocess is dead. This detects death in ~60s (one engine tick) rather than waiting hours for `timeout_at`. Additionally, store `(pid, process_start_time)` at spawn time (Linux `/proc/<pid>/stat` field 22) to detect PID reuse — if a process exists at the PID but started after the task was created, treat it as dead.

2. **Grace period: 2 minutes after PID death (configurable).** The subprocess may have exited cleanly and the callback delivery message is in-flight (network delay, queue lag). Wait `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120) of confirmed PID-death before declaring abandonment. The grace timer starts from `updated_at` (last state transition on the task), not `started_at` — this prevents false positives if the task autonomously retries or the subprocess restarts. Track "first seen dead" timestamp in task metadata.

3. **Transition to `failed`, not `expired`.** The task didn't time out — it crashed. Use `failed` with `error_reason: "subprocess_exited_without_delivery"` for audit clarity. The existing parent-reaper (`reap_orphaned_parent_tasks`) already handles `failed` callbacks correctly. Clear `timeout_at` when watchdog marks `failed` to avoid double-processing by `expire_timed_out_tasks`.

4. **Structured event + WARN log.** Emit both a WARN log entry and a structured telemetry event for programmatic consumption:
   ```json
   {
     "event": "callback_watchdog_detected_process_death",
     "task_id": "<callback_task_id>",
     "parent_task_id": "<parent_task_id>",
     "pid": 12345,
     "process_start_time": 1715000000,
     "detected_at": "2026-05-05T00:00:00Z",
     "grace_period_secs": 120,
     "new_status": "failed",
     "failure_reason": "subprocess_exited_without_delivery"
   }
   ```
   This enables mika-dev to immediately retry (if idempotent), alert operators with context, and update metrics without log scraping. Pattern follows existing watchdog event emissions in `mika-telemetry`.

5. **`timeout_at` as panic-fallback only.** Keep `estimated_duration_secs` at 7200 (2h) for `run_claude_pilot`. The watchdog is the primary detection mechanism (~2min). The existing 6h `timeout_at` serves as a panic-fallback for edge cases where PID tracking fails entirely (e.g., container restart where `/proc` state is lost). This avoids operational noise from unnecessary timeout-based restarts.

6. **Platform assumption: Linux only.** Process liveness detection uses `/proc/<pid>/stat` (Linux-specific). This is consistent with existing precedent — the codebase already assumes Linux in other process-management paths (cgroups, namespaces, `kill_orphan_processes`). No macOS/BSD support needed.

### Implementation Steps

#### Step 1: Store process start time at spawn and add watchdog function

**File:** `crates/mika-agent/src/skills/executor.rs`

When spawning the subprocess in `execute_long_running()`, read `/proc/<pid>/stat` field 22 (starttime) immediately after spawn and store it in the callback task's metadata as `process_start_time`:
```rust
let start_time = read_process_start_time(child.id());
db.set_task_metadata_field(&callback_task_id, "process_start_time", &start_time.to_string()).await?;
```

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Add a new periodic function `check_callback_process_liveness()` that:
1. Queries all callback tasks in `in_progress` that have a `process_id` (skip `pending` — they haven't spawned yet)
2. For each, checks if process is alive:
   - `kill(pid, 0) == -1 && errno == ESRCH` → definitely dead
   - `kill(pid, 0) == 0` → check `/proc/<pid>/stat` field 22 against stored `process_start_time`. Mismatch → PID reused, treat as dead.
3. If dead and no `metadata.first_dead_at`:
   - Set `metadata.first_dead_at = now()` via `db.set_task_metadata_field()`
4. If dead and `metadata.first_dead_at` is > `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120) ago:
   - Re-check task status (guard against race with in-flight callback delivery)
   - If still `in_progress`: mark task `failed` with `error_reason = "subprocess_exited_without_delivery"`
   - Clear `timeout_at` on the task to prevent double-processing
   - Emit structured event `callback_watchdog_detected_process_death`
   - Log WARN with callback_id, parent_task_id, pid, process_start_time
5. If alive (confirmed same process), clear `metadata.first_dead_at` if set (defensive)

#### Step 2: Wire into engine tick loop

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Call `check_callback_process_liveness()` every 60 ticks (same cadence as `expire_timed_out_tasks`). Place it after `expire_timed_out_tasks` in the tick sequence.

#### Step 3: Add DB helper for querying active callbacks with PIDs

**File:** `crates/mika-agent/src/db.rs`

```rust
pub async fn get_active_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
    // SELECT * FROM tasks
    // WHERE trigger_type = 'callback'
    //   AND status IN ('pending', 'in_progress')
    //   AND process_id IS NOT NULL
    //   AND agent_id = ?
}
```

#### Step 4: Add metadata update helper

**File:** `crates/mika-agent/src/db.rs`

```rust
pub async fn set_task_metadata_field(&self, task_id: &str, key: &str, value: &str) -> Result<()> {
    // UPDATE tasks SET metadata = json_set(metadata, '$.' || key, value)
    // WHERE id = ?
}
```

#### Step 5: Add process liveness utility with PID reuse detection

**File:** `crates/mika-agent/src/task_engine/process.rs` (new or extend existing)

```rust
use std::fs;

/// Check if a process is alive AND is the same process we spawned.
/// Returns `true` if alive and same process, `false` if dead or PID reused.
pub fn is_same_process_alive(pid: u32, expected_start_time: u64) -> bool {
    // First: is anything running at this PID?
    let signal_result = unsafe { libc::kill(pid as i32, 0) };
    if signal_result == -1 {
        return false; // ESRCH or EPERM — process gone
    }
    // Second: is it the SAME process? Check /proc/<pid>/stat field 22
    match read_process_start_time(pid) {
        Some(actual_start_time) => actual_start_time == expected_start_time,
        None => false, // /proc entry gone between kill(0) and read — race, treat as dead
    }
}

/// Read process start time from /proc/<pid>/stat (field 22, 0-indexed from after comm).
/// Returns None if process doesn't exist or /proc is unreadable.
pub fn read_process_start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    // Field 22 is starttime (clock ticks since boot). Parse after closing paren of comm field.
    let after_comm = stat.rfind(')')? + 2; // skip ") "
    let fields: Vec<&str> = stat[after_comm..].split_whitespace().collect();
    // Field 22 is index 19 after the comm closure (fields 3-52 are at indices 0-49)
    fields.get(19)?.parse().ok()
}
```

#### Step 6: Add configuration support

**File:** `crates/mika-common/src/config.rs`

Add `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` to Settings (default 120). Exposed via `settings.callback_watchdog_grace_period_secs()`.

### File Changes Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/engine.rs` | Add `check_callback_process_liveness()`, wire into tick loop |
| `crates/mika-agent/src/skills/executor.rs` | Store `process_start_time` in callback task metadata at spawn |
| `crates/mika-agent/src/db.rs` | Add `get_active_callback_tasks_with_pid()`, `set_task_metadata_field()` |
| `crates/mika-agent/src/task_engine/process.rs` | Add `is_same_process_alive()`, `read_process_start_time()` |
| `crates/mika-common/src/config.rs` | Add `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` setting (default 120) |

### Testing Strategy

1. **Unit test:** Mock a callback task with a dead PID → verify it transitions to `failed` after grace period
2. **Integration test:** Spawn a subprocess that exits immediately without callback delivery → verify engine marks it failed within ~3 minutes (2min grace + next tick)
3. **Regression:** Existing `expire_timed_out_tasks` tests still pass (this is additive, not replacing)

### Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| PID reuse (OS assigns same PID to unrelated process) | Store `process_start_time` at spawn; compare against `/proc/<pid>/stat` field 22 on every liveness check. Mismatch → treat as dead regardless of `kill(pid, 0)` result. |
| Race: callback delivers during grace period | Re-check task status immediately before marking `failed` — if it transitioned to `delivered` during grace, skip. Atomic CAS-style update: `UPDATE ... WHERE status = 'in_progress'`. |
| Non-Linux platforms | Acknowledged: `/proc` is Linux-specific. Consistent with existing codebase assumptions (cgroups, namespaces, `kill_orphan_processes`). No macOS/BSD support needed. |
| Double-processing with `timeout_at` | Clear `timeout_at` on the task when watchdog marks `failed`. `expire_timed_out_tasks` skips tasks without `timeout_at`. |
| Container restart loses `/proc` state | Panic-fallback: existing `timeout_at` (6h) still fires. Acceptable because container restarts also reset the engine — stale callbacks from previous container lifetime are not carried over (SQLite is fresh or restored from backup with no active callbacks). |

### Interaction with Existing Mechanisms

| Mechanism | Relationship to watchdog |
|-----------|--------------------------|
| `expire_timed_out_tasks` | Panic-fallback. Fires at `timeout_at` (6h). Watchdog clears `timeout_at` on detection to prevent double-fire. |
| `kill_orphan_processes` | Complementary. Fires on already-expired tasks to SIGTERM stragglers. Watchdog detects death *before* expiry. |
| `reap_orphaned_parent_tasks` | Downstream consumer. After watchdog marks callback `failed`, reaper handles the orphaned parent (marks parent `failed` if no PR URL after grace). |
| `dispatch_undelivered_callbacks` | No interaction. Only processes tasks that have results to deliver. Watchdog handles the case where no result will ever arrive. |

## Acceptance Verification

- ✅ Stalled subprocess auto-clears after ~2min (PID watchdog) — configurable via `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`
- ✅ Panic-fallback at existing `timeout_at` (6h) for edge cases where PID tracking fails
- ✅ WARN log names abandoned callback_id, parent_task_id, pid, and process_start_time
- ✅ Structured telemetry event `callback_watchdog_detected_process_death` emitted for programmatic consumption
- ✅ Subsequent dispatches succeed without manual intervention (global guard unblocked by `failed` transition)
