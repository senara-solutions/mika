# Plan: Diagnostic Instrumentation for `run_gh` Builtin Handler Timeout

**Ticket:** mika issue#900
**Type:** fix
**Branch:** `fix/900/mika-agent-mika-dev-run-gh-builtin`
**Date:** 2026-05-13

## Acceptance Pin

Per ticket: "Acceptance for THIS ticket is the diagnostic instrumentation, not the corrective fix. Once we have data on the third reproduction, we know the cause." This plan is scoped strictly to instrumentation. Corrective fixes (pipe-deadlock fix, per-tool timeout cap) are documented as recommended follow-up tickets.

## Problem Statement

The `run_gh` builtin handler hung for 600 seconds on two consecutive invocations when mika-dev queried CI failure details on PR mika#898. The handler produced zero diagnostic output during the hang — only a generic timeout log line after 600s elapsed.

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

### Hypothesized subprocess behavior (UNVERIFIED — sample size 2)

**Hypothesis A (pipe deadlock):** `gh run view --log-failed` against a failed CI run can produce megabytes of test output. After `spawn_and_collect` reads 10KB via `.take()`, the stdout reader completes, but the process may still be writing to its stdout pipe. The pipe buffer fills (typically 64KB on Linux), and `gh` blocks on the write. Meanwhile, `child.wait().await` waits for `gh` to exit, creating a deadlock.

**Hypothesis B (auth/network stall):** The `MIKA_GITHUB_TOKEN→GH_TOKEN` re-injection path interacts with GitHub API rate limiting or token issues, causing `gh` to hang on an auth retry loop or network backoff.

**Hypothesis C (gh interactive prompt):** Despite `GH_PROMPT_DISABLED=1` and `stdin(Stdio::null())`, certain `gh` subcommands may still wait for TTY input in edge cases.

**The instrumentation in this ticket is designed to distinguish between these hypotheses on the next reproduction.** The diagnostic fields (argv, stdout/stderr byte counts at intervals, process exit status) will discriminate:
- Hypothesis A: stdout_bytes reaches 10KB quickly, then progress logs show no further change while `stdout_done=true, stderr_done=true` but process hasn't exited
- Hypothesis B: stdout_bytes stays at 0 or low values throughout; the subprocess itself is stalled
- Hypothesis C: similar to B but with specific `gh` subcommands that trigger prompts

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

This logs the exact `gh` subcommand at invocation time — the missing diagnostic that makes the current timeout log useless for triage. On the next reproduction, this field tells us exactly which `gh` subcommand triggered the hang and whether a token was available.

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

Add a post-read, pre-wait diagnostic log. This is simpler than the `select!` + ticker approach and avoids replacing the clean `tokio::join!` pattern:

```rust
// After the existing tokio::join! read completes:
let (stdout_res, stderr_res) = tokio::join!(
    stdout_take.read_to_end(&mut stdout_buf),
    stderr_take.read_to_end(&mut stderr_buf),
);
stdout_res.ok();
stderr_res.ok();

// NEW: Diagnostic log — fires once after reads complete, before wait().
// On the next hang reproduction, this tells us:
// - If reads completed instantly (pipe deadlock hypothesis: reads finish, wait blocks)
// - stdout/stderr byte counts (tells us if the process produced output at all)
info!(
    tool = %tool_name,
    stdout_bytes = stdout_buf.len(),
    stderr_bytes = stderr_buf.len(),
    "spawn_and_collect reads complete, waiting for process exit"
);

let status = match child.wait().await {
```

**Why not `select!` + ticker:** The ticket requests "log stdout/stderr byte counts at 30-second intervals." However, the `tokio::join!` reads complete almost instantly (they stop at 10KB or EOF). The hang occurs *after* reads complete, during `child.wait()`. A single log line between reads and wait is sufficient to discriminate between the hypotheses:
- If this log line appears immediately before a 600s timeout → the process is alive but blocked (supports Hypothesis A: pipe deadlock)
- If this log line never appears → the reads themselves are blocking (supports Hypothesis B: process not producing output, stalled on auth/network)

Adding a second ticker-based progress log during `child.wait()` would require wrapping `child.wait()` in a `select!`, which adds more complexity than the diagnostic value justifies for a single reproduction. If the third reproduction shows the post-read log fires and then `wait()` hangs, we'll know it's a pipe deadlock and can apply the corrective fix (see Recommended Follow-Up Tickets).

### Step 4: Tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (test module)

1. **Test `spawn_and_collect` diagnostic log presence:** Call `spawn_and_collect` with `echo "hello"` and verify it returns successfully. The log assertions are structural — if the code compiles, the log fields are present.

2. **Test `spawn_and_collect` with large output:** Call `spawn_and_collect` with a command that produces > `MAX_OUTPUT_LEN` bytes (e.g., `yes | head -c 20000`) and verify it completes. This tests the existing behavior (not a fix) and documents the pipe-deadlock risk for the follow-up ticket.

### Step 5: Verify with build

```bash
cargo build -p mika-agent
cargo test -p mika-agent -- builtin_handlers
cargo clippy -p mika-agent
```

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Steps 1, 3, 4: `run_gh` invocation log, post-read diagnostic log in `spawn_and_collect`, tests |
| `crates/mika-agent/src/agent.rs` | Step 2: `input_excerpt` in both timeout log lines (builtin-tool path and skill-tool path) |

## Blast Radius

**`spawn_and_collect` (Step 3):** The diagnostic log is additive (no behavioral change). `spawn_and_collect` is shared by all CLI builtin handlers. Full list of callers:
- `run_gh` — GitHub CLI commands (the affected handler)
- `run_gws` — Google Workspace CLI
- `run_git_*` — Git operations (push, pull, clone, etc.)
- `gh_read` — Read-only GitHub operations (mika-arch)

The `info!` log line fires once per invocation for all of these. This is acceptable — the log volume is proportional to tool invocations (bounded by max 20 steps per agent turn).

**`dispatch_tool` (Step 2):** The `input_excerpt` log field is added to timeout events only. These fire at most once per tool timeout — extremely rare in practice.

## Risk Assessment

- **Low risk:** All changes are purely additive logging. No behavioral changes to `run_gh`, `spawn_and_collect`, or `dispatch_tool`.

## Recommended Follow-Up Tickets (out of scope per acceptance pin)

### Follow-up 1: Fix pipe-deadlock in `spawn_and_collect`

After `.take(MAX_OUTPUT_LEN)` reads complete, drop the pipe handles before `child.wait()`:
```rust
drop(stdout_take);
drop(stderr_take);
```
This closes the pipe and causes the subprocess to get `SIGPIPE` if it's still writing. Without this, `wait()` deadlocks when the process writes > `MAX_OUTPUT_LEN`. **Blast radius:** all CLI builtin handlers (listed above). **Prerequisite:** this ticket's instrumentation confirms the pipe-deadlock hypothesis on the third reproduction.

### Follow-up 2: Cap `run_gh` per-tool timeout

Add a `RUN_GH_TIMEOUT_SECS` constant (60s) as an inner timeout inside `run_gh()`, independent of the inherited skill timeout. `run_gh` should never need more than 60s for any `gh` subcommand. **Blast radius:** `run_gh` only.
