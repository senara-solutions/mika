# Plan: Cancel Task Kills Associated Process (#855)

**Ticket:** senara-solutions/mika#855
**Depends on:** #854 (closed — `process_id` column exists, `set_task_process_id`/`clear_task_process_id` wired)
**Branch:** `feat/855/cancelling-a-task-kills-the-associated`

## Problem

Cancelling a task today flips the DB status to `cancelled` and **does** send SIGTERM/SIGKILL to the `claude-pilot` process via `cancel_task_and_kill()` in `process_kill.rs`. However, two gaps remain:

1. **No process group isolation at spawn.** `executor.rs` spawns `claude-pilot` without `setsid()` or `process_group(0)`, so the child inherits the parent's (mika-server's) process group. The `kill(-pid, SIGTERM)` call in `kill_process_gracefully()` sends to process group `pid` — which doesn't exist because `claude-pilot` is not a group leader. The fallback `kill(pid, SIGTERM)` only kills `claude-pilot` itself; Claude Code (its child) continues as an orphan consuming tokens.

2. **No PID reuse guard in the cancel path.** `kill_process_gracefully()` uses a basic `/proc/<pid>/stat` existence check, but does NOT compare process start times. The callback watchdog (`engine.rs`) correctly uses `is_same_process_alive(pid, expected_start_time)`, but the cancel path doesn't — creating a TOCTOU window where cancel could kill a wrong process if PID was reused between task death and operator-initiated cancel.

## Scope

### In scope
- Add process group creation at the spawn site so `kill(-pid, ...)` reliably kills the entire process tree
- Add PID reuse guard to the cancel/kill path using stored `process_start_time` from task metadata
- Add tests for the new behavior

### Out of scope
- Dashboard cancel button (separate UI ticket)
- Non-Linux platforms (existing convention: process kill is Linux-only, macOS fallback is best-effort)

## Implementation

### Step 1: Process group isolation at spawn (`executor.rs`)

**File:** `crates/mika-agent/src/skills/executor.rs` (~line 1898)

Add `process_group(0)` to the `Command` builder in `spawn_long_running_exec()`. This makes `claude-pilot` the leader of a new process group (PGID == PID), so `kill(-pid, SIGTERM)` correctly signals the entire tree (claude-pilot + Claude Code + any grandchildren).

```rust
use std::os::unix::process::CommandExt;

// In spawn_long_running_exec(), after existing cmd setup:
cmd.process_group(0);  // Make child a process group leader (#855)
```

**Why `process_group(0)` over `pre_exec(setsid)`:**
- `process_group(0)` is stable since Rust 1.64, our minimum is 1.91
- No `unsafe` block required (unlike `pre_exec`)
- `setsid()` also detaches from the controlling terminal, which is unnecessary — we only need process group isolation

**Risk:** None. `process_group(0)` is a standard POSIX pattern. The child's stdio is already redirected (stdin piped, stdout null, stderr piped), so terminal detachment isn't relevant. `kill_on_drop(false)` is already set, so the process outlives the `Command` handle as intended.

### Step 2: PID reuse guard in `kill_process_gracefully` (`process_kill.rs`)

**File:** `crates/mika-agent/src/task_engine/process_kill.rs`

**2a.** Change the signature of `kill_process_gracefully` to accept an optional `expected_start_time`:

```rust
pub async fn kill_process_gracefully(pid: i64, expected_start_time: Option<u64>) -> bool
```

**2b.** Replace the `is_process_alive(pid)` early-exit check with a start-time-aware check when `expected_start_time` is `Some`:

```rust
if let Some(expected) = expected_start_time {
    if !process_liveness::is_same_process_alive(pid as u32, expected) {
        debug!(pid, "process already dead or PID reused, skipping kill");
        return true;  // Not our process to kill
    }
} else if !is_process_alive(pid) {
    debug!(pid, "process already dead, skipping kill");
    return true;
}
```

**2c.** Update `kill_process_immediate` similarly (accepts optional start time, skips if PID reused).

**2d.** Update `cancel_task_and_kill` to extract `process_start_time` from task metadata and pass it through:

```rust
let start_time: Option<u64> = task.as_ref()
    .and_then(|t| t.metadata.as_deref())
    .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
    .and_then(|v| v.get("process_start_time")?.as_str()?.parse().ok());

// ...
let killed = kill_process_gracefully(pid, start_time).await;
```

This mirrors the exact pattern used by the callback watchdog in `engine.rs` (lines 493-514).

**Backward compat:** `expected_start_time: Option<u64>` defaults to `None` at all existing callsites. Pre-#854 tasks without stored start times fall back to the existing `/proc/<pid>/stat` existence check.

### Step 3: Update callsites

**3a.** `cancel_task_and_kill()` in `process_kill.rs` — already addressed in Step 2d.

**3b.** `kill_orphan_processes()` in `engine.rs` — update to pass `None` (orphan cleanup doesn't have start time context; existing behavior preserved).

**3c.** Any other callers of `kill_process_gracefully` or `kill_process_immediate` — audit and update. Expected: only the above two sites.

### Step 4: Tests

**4a.** Unit test in `process_kill.rs`: test that `kill_process_gracefully` with a mismatched start time returns `true` without sending signals. Spawn a real child process, record its start time, kill it, spawn a new process that reuses the PID slot (probabilistic but can be controlled), and verify the guard fires.

**4b.** Unit test in `process_kill.rs`: test that `kill_process_gracefully` with `None` start time falls back to the existing behavior (backward compat).

**4c.** Integration test or expansion of existing `cancel_task.rs` tests: verify that cancelling a task with a live process results in the process being killed. The existing four test cases in `cancel_task.rs` use mocked DB state — extend one to verify the `process_killed` outcome field.

**4d.** Verify `process_group(0)` works in the spawn path by adding a comment-documented manual test procedure in the PR description (spawn a sleep process, cancel, verify `ps` shows no orphans). This is hard to test in CI without privileged process management.

### Step 5: Update doc comment on `process_kill.rs`

Update the module-level doc comment (lines 1-10) to reflect that PID reuse risk is now mitigated via start-time comparison, not just documented as a known TOCTOU window.

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/executor.rs` | Add `process_group(0)` + `use std::os::unix::process::CommandExt` |
| `crates/mika-agent/src/task_engine/process_kill.rs` | Add `expected_start_time` param to `kill_process_gracefully`/`kill_process_immediate`; extract start time in `cancel_task_and_kill`; update module doc; add tests |
| `crates/mika-agent/src/task_engine/engine.rs` | Update `kill_orphan_processes` callsite to pass `None` for start time |

## Acceptance Criteria Mapping

| AC | Implementation |
|----|----------------|
| Cancelling a running task kills `claude-pilot` and its Claude Code child within the grace window | Step 1 (`process_group(0)`) ensures `kill(-pid, SIGTERM)` reaches the entire process tree. Step 2 preserves the existing 5-second grace + SIGKILL escalation. |
| No orphaned processes after cancel | Step 1 ensures Claude Code dies with its parent process group. The existing `kill(-pid, KILL)` fallback handles stubborn children. |
| Cancelling a task with no live process is a clean no-op | Already implemented: `cancel_task_and_kill` returns `process_killed: None` when `process_id IS NULL`. Step 2 adds PID reuse guard so stale PIDs are also a no-op. |

## Risk Assessment

**Low risk.** The core kill machinery already exists and works. The two changes are additive:
- `process_group(0)` is a standard POSIX call with no side effects beyond process group isolation
- The start-time guard is a strictly tighter check (fewer false kills, not more)

No schema migration. No API changes. No breaking changes to tool output shapes.
