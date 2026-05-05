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

1. **Detect by PID liveness, not by timeout.** The subprocess PID is stored in `tasks.process_id`. If `kill(pid, 0)` fails with ESRCH (no such process), the subprocess is dead. This detects death in ~60s (one engine tick) rather than waiting hours for `timeout_at`.

2. **Grace period: 2 minutes after PID death.** The subprocess may have exited cleanly and the callback delivery message is in-flight (network delay, queue lag). Wait 2 minutes of confirmed PID-death before declaring abandonment. Track "first seen dead" timestamp in task metadata.

3. **Transition to `failed`, not `expired`.** The task didn't time out — it crashed. Use `failed` with `error_reason: "subprocess_exited_without_delivery"` for audit clarity. The existing parent-reaper (`reap_orphaned_parent_tasks`) already handles `failed` callbacks correctly.

4. **WARN log entry.** Emit structured log: `callback_id`, `parent_task_id`, `process_id`, `first_dead_at`, `declared_dead_at`. Satisfies acceptance criterion #2.

5. **Configurable timeout (secondary).** Also reduce the `timeout_at` ceiling for `run_claude_pilot` callbacks to a configurable agent-level default (default 30m). This provides a belt-and-suspenders backstop for edge cases where PID tracking fails (e.g., PID reuse on long-lived systems). The 30m value matches the ticket's acceptance criterion.

### Implementation Steps

#### Step 1: Add `first_dead_at` to task metadata

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Add a new periodic function `check_callback_process_liveness()` that:
1. Queries all callback tasks in `pending`/`in_progress` that have a `process_id`
2. For each, checks if process is alive via `unsafe { libc::kill(pid, 0) }`
3. If dead and no `metadata.first_dead_at`:
   - Set `metadata.first_dead_at = now()` via `db.update_task_metadata()`
4. If dead and `metadata.first_dead_at` is > 120s ago:
   - Mark task `failed` with `error_reason = "subprocess_exited_without_delivery"`
   - Log WARN with callback_id, parent_task_id, freed task info
5. If alive, clear `metadata.first_dead_at` if set (process recovered — shouldn't happen but defensive)

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

#### Step 5: Reduce default callback timeout for dev-pilot

**File:** `skills/bundled/dev-pilot/tools.json`

Change the `estimated_duration_secs` for `run_claude_pilot` from 7200 to 600 (10 minutes). This yields a `timeout_at` of 1800s (30 minutes) via the `× 3` multiplier — matching the ticket's acceptance criterion exactly.

#### Step 6: Add process liveness utility

**File:** `crates/mika-agent/src/task_engine/process.rs` (new or extend existing)

```rust
pub fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
```

### File Changes Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/engine.rs` | Add `check_callback_process_liveness()`, wire into tick loop |
| `crates/mika-agent/src/db.rs` | Add `get_active_callback_tasks_with_pid()`, `set_task_metadata_field()` |
| `crates/mika-agent/src/task_engine/process.rs` | Add `is_process_alive()` (may extend existing `process_kill` module) |
| `skills/bundled/dev-pilot/tools.json` | Reduce `estimated_duration_secs` to 600 |

### Testing Strategy

1. **Unit test:** Mock a callback task with a dead PID → verify it transitions to `failed` after grace period
2. **Integration test:** Spawn a subprocess that exits immediately without callback delivery → verify engine marks it failed within ~3 minutes (2min grace + next tick)
3. **Regression:** Existing `expire_timed_out_tasks` tests still pass (this is additive, not replacing)

### Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| PID reuse (OS assigns same PID to unrelated process) | 2-minute grace period + check that the PID's start time matches. On Linux, `/proc/<pid>/stat` field 22 is start time. If we detect the process is "alive" but started after the task was created, treat as dead. |
| Race: callback delivers during grace period | Check task status before marking failed — if it transitioned to `delivered` during grace, skip. |
| Non-Linux platforms | `libc::kill` works on all Unix. Windows not supported (mika runs on Linux). |

## Acceptance Verification

- ✅ Stalled subprocess auto-clears after ~2min (PID watchdog) or 30min (timeout_at backstop)
- ✅ WARN log names abandoned callback_id and freed task_id
- ✅ Subsequent dispatches succeed without manual intervention (global guard unblocked by `failed` transition)
