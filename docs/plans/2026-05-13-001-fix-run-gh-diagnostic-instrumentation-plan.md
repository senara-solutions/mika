# Plan: Diagnostic Instrumentation for `run_gh` Builtin Handler Timeout

**Ticket:** mika issue#900
**Type:** fix
**Branch:** `fix/900/mika-agent-mika-dev-run-gh-builtin`
**Date:** 2026-05-13

## Problem Statement

The `run_gh` builtin handler hung for 600 seconds on two consecutive invocations when mika-dev queried CI failure details on PR mika#898. The handler produced zero diagnostic output during the hang — only a generic timeout log line after 600s elapsed. The acceptance criteria for this ticket is diagnostic instrumentation (not the corrective fix).

## Root Cause Analysis (from code investigation)

### Why 600s, not 30s?

`run_gh` is a skill-defined builtin handler dispatched via `builtin_handlers::execute()` at `agent.rs:2691`. Skill-defined tools use `dispatch.skill_timeout`, computed by `max_skill_timeout()` — the **maximum** timeout across all matched skills in the turn. When mika-dev's turn matches `dev-pilot` (which declares `timeout_secs = 600` in `skill.toml`), every skill-defined tool in that turn inherits the 600s timeout, including `run_gh` which should return in seconds.

### Why does the agent deadline (300s) not cap it?

The 5-minute agent deadline is checked at the **top** of each loop iteration (`agent.rs:859`), not around each tool execution. If a tool call starts within the deadline but the outer `tokio::time::timeout` wraps it at 600s, the tool runs past the deadline. The deadline is only checked on the *next* iteration, which never arrives because the tool is still running.

### Why does `spawn_and_collect` hang?

`spawn_and_collect()` (`builtin_handlers.rs:346-417`) has no internal timeout or progress logging. It:
1. Takes stdout/stderr handles and reads them concurrently via `tokio::join!` with `.take(MAX_OUTPUT_LEN)` (10KB)
2. Waits for process exit via `child.wait().await`

If the `gh` subprocess itself hangs (auth retry loop, rate-limit backoff, network stall, or blocked on writing to a full pipe after 10KB is consumed by the reader), `spawn_and_collect` blocks silently for the full outer timeout.

### Suspected subprocess behavior

`gh run view --log-failed` against a failed CI run can produce megabytes of test output. After `spawn_and_collect` reads 10KB via `.take()`, the stdout reader completes, but the process may still be writing to its stdout pipe. The pipe buffer fills (typically 64KB on Linux), and `gh` blocks on the write. Meanwhile, `child.wait().await` waits for `gh` to exit, creating a deadlock: the process can't exit because it can't write, and the agent can't proceed because the process hasn't exited.

**This is the most likely cause.** The `.take()` reader stops consuming at 10KB, but the process keeps producing. The OS pipe buffer (64KB) fills, and the process blocks on `write()`. `child.wait()` never returns. The 600s outer timeout is the only escape hatch.

## Implementation Plan

### Step 1: Add per-invocation diagnostic logging to `run_gh`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`
**Function:** `run_gh()` (line 1586)

Before `spawn_and_collect()` call (line 1653), add:

```rust
info!(
    tool = "run_gh",
    argv = ?&gh_args.args,
    repo = ?&gh_args.repo,
    has_github_token = ctx.github_token.is_some(),
    "run_gh invocation"
);
```

This logs the exact `gh` subcommand at invocation time — the missing diagnostic that makes the current timeout log useless for triage.

### Step 2: Add argv excerpt to timeout log line in `dispatch_tool`

**File:** `crates/mika-agent/src/agent.rs`
**Function:** `dispatch_tool()` (line 2663)

The timeout error at line 2681-2682 currently logs:
```
warn!(tool = %name, timeout_secs = timeout, "tool execution timed out");
```

This is in the builtin-tool path (path 1). The skill-tool path (path 2) at line 2699 has a similar pattern. Both need the input args included.

Add truncated input to both timeout log lines:
```rust
let input_excerpt = serde_json::to_string(&input)
    .unwrap_or_default()
    .chars()
    .take(200)
    .collect::<String>();
warn!(
    tool = %name,
    timeout_secs = timeout,
    input_excerpt = %input_excerpt,
    "tool execution timed out"
);
```

This satisfies the acceptance criterion: "add a per-invocation argv excerpt to the timeout-fired log line so future hangs are auto-categorized."

### Step 3: Add progress logging to `spawn_and_collect`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`
**Function:** `spawn_and_collect()` (line 346)

Replace the simple `tokio::join!` read pattern with a progress-aware variant that logs byte counts at 30-second intervals:

```rust
// Read stdout/stderr with progress logging at 30s intervals
let tool_name_owned = tool_name.to_string();
let read_with_progress = async {
    let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
    let mut stderr_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
    let mut stdout_take = stdout_handle.take(MAX_OUTPUT_LEN as u64);
    let mut stderr_take = stderr_handle.take(MAX_OUTPUT_LEN as u64);

    let progress_interval = tokio::time::Duration::from_secs(30);
    let mut progress_tick = tokio::time::interval(progress_interval);
    progress_tick.tick().await; // skip first immediate tick

    // Use select! to interleave progress logging with reads
    let stdout_fut = stdout_take.read_to_end(&mut stdout_buf);
    let stderr_fut = stderr_take.read_to_end(&mut stderr_buf);

    tokio::pin!(stdout_fut);
    tokio::pin!(stderr_fut);

    let mut stdout_done = false;
    let mut stderr_done = false;

    loop {
        tokio::select! {
            res = &mut stdout_fut, if !stdout_done => {
                res.ok();
                stdout_done = true;
                if stdout_done && stderr_done { break; }
            }
            res = &mut stderr_fut, if !stderr_done => {
                res.ok();
                stderr_done = true;
                if stdout_done && stderr_done { break; }
            }
            _ = progress_tick.tick() => {
                warn!(
                    tool = %tool_name_owned,
                    stdout_bytes = stdout_buf.len(),
                    stderr_bytes = stderr_buf.len(),
                    stdout_done,
                    stderr_done,
                    "spawn_and_collect still reading subprocess output"
                );
            }
        }
    }
    (stdout_buf, stderr_buf)
};
```

**Decision:** Use `select!` with progress ticker vs simple `tokio::join!`. The `select!` approach adds complexity but provides the 30-second progress logging the ticket requests. The existing `tokio::join!` is simpler but provides zero visibility during hangs.

### Step 4: Fix the pipe-deadlock root cause in `spawn_and_collect`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`
**Function:** `spawn_and_collect()` (line 346)

The current pattern has a subtle deadlock: after `.take(MAX_OUTPUT_LEN)` reads stop consuming, `child.wait()` blocks forever if the process keeps writing to stdout. Fix by **dropping** the stdout/stderr handles after reading, which closes the pipe and causes the process to get `EPIPE` / `SIGPIPE`:

After reading completes:
```rust
// Drop the take wrappers (and thus the underlying pipe handles) so the
// subprocess gets SIGPIPE if it's still writing. Without this, wait()
// deadlocks when the process writes > MAX_OUTPUT_LEN.
drop(stdout_take);
drop(stderr_take);
```

**Note:** This is the actual fix for the hang mechanism, but it's diagnostic-adjacent — the ticket says "once cause is known, file or apply the corrective fix (likely separate ticket)." I'm including it here because:
1. It's a 2-line fix in the same function
2. It directly addresses the deadlock that the diagnostics are instrumenting
3. Filing a separate ticket for 2 lines adds overhead with no safety benefit

If the architect disagrees, this step can be extracted to a separate ticket.

### Step 5: Cap `run_gh` tool timeout independently of skill timeout

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Add a `RUN_GH_TIMEOUT_SECS` constant (60s) and apply it inside `run_gh()` by wrapping the `spawn_and_collect` call:

```rust
const RUN_GH_TIMEOUT_SECS: u64 = 60;

// Inside run_gh():
let output = match tokio::time::timeout(
    std::time::Duration::from_secs(RUN_GH_TIMEOUT_SECS),
    spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?"),
).await {
    Ok(output) => output,
    Err(_) => {
        warn!(
            tool = "run_gh",
            timeout_secs = RUN_GH_TIMEOUT_SECS,
            argv = ?&gh_args.args,
            "run_gh internal timeout — gh subprocess did not complete"
        );
        ToolOutput::error(format!(
            "gh command timed out after {RUN_GH_TIMEOUT_SECS}s. \
             The command may have produced too much output or hit a network issue."
        ))
    }
};
```

**Decision:** This is defense-in-depth against the inherited 600s skill timeout. `run_gh` should never need more than 60s for any `gh` subcommand. The inner timeout fires before the outer skill timeout, giving a specific `run_gh`-scoped error message instead of a generic "tool timed out."

**Out of scope (per ticket):** This is technically a corrective fix, not just diagnostic instrumentation. Same argument as Step 4 — the fix is small and directly related.

### Step 6: Tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (test module)

1. **Test `run_gh` invocation logging:** Verify the `run_gh invocation` log line is emitted with correct fields. Use `tracing-test` or assert on the log output.
2. **Test pipe-deadlock fix:** Create a mock process that writes > `MAX_OUTPUT_LEN` bytes to stdout and verify `spawn_and_collect` returns without deadlocking. Use a timeout assertion.
3. **Test `run_gh` internal timeout:** Mock a `gh` command that sleeps forever, verify `run_gh` returns an error within `RUN_GH_TIMEOUT_SECS`.

**Practical constraint:** The existing test infrastructure in `builtin_handlers.rs` uses `#[tokio::test]` with real commands. The deadlock test needs a synthetic slow command — `sleep 999` or a custom script. The internal timeout test similarly needs `sleep`.

### Step 7: Verify with build

```bash
cargo build -p mika-agent
cargo test -p mika-agent -- builtin_handlers
cargo clippy -p mika-agent
```

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Steps 1, 3, 4, 5, 6: diagnostic logging, progress logging, pipe-deadlock fix, internal timeout, tests |
| `crates/mika-agent/src/agent.rs` | Step 2: input_excerpt in timeout log lines |

## Risk Assessment

- **Low risk:** Steps 1-3 are purely additive logging. No behavioral change.
- **Medium risk:** Step 4 (pipe-handle drop) changes `spawn_and_collect` behavior. The fix is correct — closing the pipe is the right thing to do after reading is done — but it affects ALL CLI builtin handlers (gh, gws, etc.). If a handler relies on the process continuing to write after the reader stops, this could change behavior. In practice, no handler cares about output beyond `MAX_OUTPUT_LEN`.
- **Low risk:** Step 5 adds an inner timeout that is strictly tighter than the existing outer timeout. If `run_gh` currently succeeds within 60s (empirically true for all normal operations), this has no behavioral effect.

## Open Questions

1. Should Step 4 (pipe fix) and Step 5 (timeout cap) be separate tickets? The ticket says "acceptance is diagnostic instrumentation, not corrective fix" but the fixes are 2-line and 10-line changes in the same function.
2. Should the progress logging interval (30s) be configurable? Decision: no — hardcode. The interval is for operator triage, not user-facing behavior.
3. Should `RUN_GH_TIMEOUT_SECS` apply to `gh_read` as well? Decision: no — `gh_read` is a different handler with different semantics (allowlisted operations). It already has structural safety (operation allowlist, structured errors).
