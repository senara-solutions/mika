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
// Ticket acceptance: "environment variable set (with token redaction)" — emit
// the set of env var keys this handler is about to set on the subprocess
// (KEYS only, never values). Threaded through a literal const because the
// callsite knows exactly which keys are conditionally set; no scan of cmd.env.
let env_keys_set: &[&str] = match ctx.github_token {
    Some(_) => &["GH_PROMPT_DISABLED", "GH_TOKEN"],
    None => &["GH_PROMPT_DISABLED"],
};
info!(
    tool = "run_gh",
    argv = ?&gh_args.args,
    repo = ?&gh_args.repo,
    env_keys_set = ?env_keys_set,
    has_github_token = ctx.github_token.is_some(),
    "run_gh invocation"
);
```

This logs the exact `gh` subcommand at invocation time — the missing diagnostic that makes the current timeout log useless for triage. On the next reproduction this gives us:
- **argv** — the precise `gh` subcommand and flag set
- **env_keys_set** — the literal env var keys the handler injected (`["GH_PROMPT_DISABLED", "GH_TOKEN"]` or `["GH_PROMPT_DISABLED"]`), satisfying the ticket's "environment variable set (with token redaction)" wording — names only, never values
- **has_github_token** — boolean discriminator: token re-injected (mika#515 path) vs PAT fallback

Token redaction is structural: the literal `env_keys_set` slice contains only `&'static str` keys we control. The actual token value is never read by the log statement and never reaches the tracing layer. Defense-in-depth: the existing `Settings::Debug` impl + `SecretString` already redacts elsewhere; this codepath simply never touches the value.

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

The ticket spec is literal: "subprocess stdout/stderr byte counts at 30-second intervals." The plan implements both a post-read transition log AND a 30-second periodic progress log during `child.wait()`. The two surfaces are complementary and the architect-flagged spec deviation (F2) is removed.

**Architecture:**

1. **Reader rewrite:** replace `read_to_end` (atomic read) with a chunked read loop that increments shared `Arc<AtomicUsize>` counters per chunk. This is what lets a separate progress task observe byte counts during the read itself. Chunk size 256 bytes — small enough that counters update before bounded `MAX_OUTPUT_LEN` (10KB) cap fires, large enough to keep syscall overhead negligible.

2. **Progress ticker:** `tokio::time::interval(Duration::from_secs(30))` task that snapshots both counters every 30s. The ticker's first tick is consumed (`interval.tick().await`) before entering the periodic loop so we don't double-fire at t=0. The ticker is `tokio::spawn`ed and `.abort()`ed on completion.

3. **Coordination:** `tokio::select!` on `(both reads complete + child.wait())` vs ticker iteration. The reads and child.wait are joined; the ticker is the loser arm that re-enters the loop. When reads + child.wait win, the ticker is aborted and we proceed to the existing exit-status branch.

```rust
let stdout_count = Arc::new(AtomicUsize::new(0));
let stderr_count = Arc::new(AtomicUsize::new(0));
let started_at = Instant::now();

// Chunked read tasks: increment counters per chunk
let stdout_reader = tokio::spawn(read_with_counter(
    stdout_handle, MAX_OUTPUT_LEN, Arc::clone(&stdout_count)
));
let stderr_reader = tokio::spawn(read_with_counter(
    stderr_handle, MAX_OUTPUT_LEN, Arc::clone(&stderr_count)
));

// Progress ticker: every 30s, log a snapshot
let progress_task = {
    let stdout_count = Arc::clone(&stdout_count);
    let stderr_count = Arc::clone(&stderr_count);
    let tool_name = tool_name.to_string();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await; // discard immediate first tick
        loop {
            interval.tick().await;
            info!(
                tool = %tool_name,
                stdout_bytes = stdout_count.load(Ordering::Relaxed),
                stderr_bytes = stderr_count.load(Ordering::Relaxed),
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "spawn_and_collect progress"
            );
        }
    })
};

// Wait for both reads + process exit
let (stdout_buf, stderr_buf, status) = tokio::join!(
    stdout_reader, stderr_reader, child.wait()
);
progress_task.abort(); // ticker stops on normal completion

// Post-completion summary (complements progress ticker — fires once at end)
info!(
    tool = %tool_name,
    stdout_bytes = stdout_count.load(Ordering::Relaxed),
    stderr_bytes = stderr_count.load(Ordering::Relaxed),
    elapsed_ms = started_at.elapsed().as_millis() as u64,
    "spawn_and_collect complete"
);
```

`read_with_counter` is a private helper:

```rust
async fn read_with_counter<R: AsyncRead + Unpin>(
    reader: R,
    max_bytes: usize,
    counter: Arc<AtomicUsize>,
) -> Vec<u8> {
    let mut take = reader.take(max_bytes as u64);
    let mut buf = Vec::with_capacity(max_bytes.min(8192));
    let mut chunk = [0u8; 256];
    while let Ok(n) = take.read(&mut chunk).await {
        if n == 0 { break; }
        buf.extend_from_slice(&chunk[..n]);
        counter.fetch_add(n, Ordering::Relaxed);
    }
    buf
}
```

**Discriminators on next reproduction:**
- **Hypothesis A (pipe deadlock):** progress ticker fires at 30s, 60s, 90s, ... with `stdout_bytes ≈ 10240` (MAX_OUTPUT_LEN ceiling) — reads completed instantly, ticker is logging the steady-state while `child.wait()` blocks
- **Hypothesis B (auth/network stall):** progress ticker fires with `stdout_bytes = 0` for the entire duration — subprocess produced no output
- **Hypothesis C (interactive prompt):** indistinguishable from B at the log level; identified by argv (Step 1) showing a subcommand that *could* prompt

**`tokio::select!` lint guard:** `scripts/check-loop-select.sh` rejects `tokio::select!` only inside `run_loop`'s body (mika#848 — the deadline-check guarantee). `spawn_and_collect` is in `builtin_handlers.rs`, outside `run_loop`. The lint does not fire. This implementation uses `tokio::join!` for reads+wait coordination and `tokio::spawn` + `.abort()` for the ticker — no `select!` is needed, sidestepping the lint surface entirely.

**Why not run_gh-only:** Co-locating the instrumentation in `spawn_and_collect` (shared by `run_gh`, `run_gws`, `gh_read`, git ops) gives every CLI subprocess the same forensic trail. Per F6 the architect confirmed the blast radius is acceptable (max 20 tool steps per turn → ≤20 invocations × `(1 + ⌈elapsed/30⌉)` log lines per turn; steady-state for fast tools is 2 log lines per invocation — start + complete). Narrowing to `run_gh` only would require duplicating the ticker machinery in a `spawn_and_collect_instrumented` variant for the sibling handlers when they hang next; we'd rather pay the cost once.

### Step 4: Tests

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs` (test module)

Address architect F5: log-capturing assertions, not just compile-time structural checks. Use `tracing_subscriber::fmt().with_test_writer()` or a layer that captures `tracing::Event` records into a shared `Vec<Event>`, then assert structurally.

1. **`test_spawn_and_collect_emits_complete_log`:** Use a tracing-capture layer; call `spawn_and_collect` with `echo "hello"`; assert one `spawn_and_collect complete` event was recorded with `stdout_bytes == 6` (echo appends newline), `stderr_bytes == 0`, `elapsed_ms > 0`.

2. **`test_run_gh_invocation_log_redacts_token`:** Capture tracing events; invoke `run_gh` with a fake token (`ctx.github_token = Some("FAKE_TOKEN_DO_NOT_LOG_ME")`); assert the captured `run_gh invocation` event:
   - includes `env_keys_set` field containing `"GH_TOKEN"` literally
   - has `has_github_token = true`
   - has NO event field whose serialized value contains the literal string `"FAKE_TOKEN_DO_NOT_LOG_ME"`

3. **`test_spawn_and_collect_handles_large_output`:** Call `spawn_and_collect` with `yes | head -c 20000` (> `MAX_OUTPUT_LEN`); assert it returns within a wall-clock budget of 10 seconds (currently this can deadlock — the test documents the pipe-deadlock risk for the follow-up ticket and serves as a regression test once the follow-up lands).

4. **`test_spawn_and_collect_progress_ticker_fires`:** Use a short-interval test override of the ticker (gated via `#[cfg(test)]` const or a builder hook) of 100ms instead of 30s; spawn a `sleep 0.5` subprocess; capture events; assert at least 3 `spawn_and_collect progress` events fired with monotonically non-decreasing `elapsed_ms`. This validates the ticker plumbing without making the test slow.

Test infrastructure: a small `capture_tracing_events()` helper in the test module that returns a `(Subscriber, Arc<Mutex<Vec<CapturedEvent>>>)` pair. Used by tests 1, 2, 4.

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

**`spawn_and_collect` (Step 3):** Diagnostic logging is additive at the observability layer; the reader rewrite (chunked-read with counter) is a structural change to subprocess collection but preserves identical output buffering and bounds (`MAX_OUTPUT_LEN`). `spawn_and_collect` is shared by all CLI builtin handlers:
- `run_gh` — GitHub CLI commands (the affected handler)
- `run_gws` — Google Workspace CLI
- `run_git_*` — Git operations
- `gh_read` — Read-only GitHub operations (mika-arch)

**Log volume per invocation:**
- 1 × `spawn_and_collect complete` event (always)
- N × `spawn_and_collect progress` events, where N = ⌊elapsed_secs / 30⌋. For fast tools (< 30s), N = 0. For a hung tool that times out at 600s, N = 19.

Worst-case turn: 20 tool steps × (1 + 19) = 400 log lines. Realistic steady-state: 20 tool steps × 1 = 20 log lines. Log layer is `info!` (not debug) — fine for the production sink. Acceptable per architect F6.

**Behavioral change risk:** The chunked-read loop replaces a single `read_to_end` call. Failure modes covered:
- Error returned by `take.read()` mid-stream → loop exits via `Err` arm; partial buffer returned (matches existing behavior on read errors).
- Counter `Ordering::Relaxed` — atomicity is per-store; we never read+modify, only fetch_add. No memory-ordering hazard.
- Progress-task abort → `tokio::spawn` futures cancel cleanly; no resource leak.

**`run_gh` (Step 1):** One additional `info!` per invocation (bounded by max 20 steps/turn).

**`dispatch_tool` (Step 2):** `input_excerpt` log field added to timeout events only. Fires at most once per tool timeout — extremely rare in practice.

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
