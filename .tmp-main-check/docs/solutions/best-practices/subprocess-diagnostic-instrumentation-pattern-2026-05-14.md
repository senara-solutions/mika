---
title: "Subprocess diagnostic instrumentation pattern for CLI builtin handlers"
module: agent
date: 2026-05-14
problem_type: best_practice
component: tooling
severity: medium
tags: [spawn-and-collect, run-gh, timeout, diagnostic, instrumentation, subprocess, progress-ticker, tracing]
applies_when: "A CLI builtin handler (run_gh, run_gws, gh_read, git ops) hangs or times out without producing actionable diagnostic output"
---

# Subprocess diagnostic instrumentation pattern for CLI builtin handlers

## Context

The `run_gh` builtin handler hung for the full 600-second timeout on two consecutive invocations (mika#900). The only log output was a generic timeout warning with no information about what `gh` subcommand was running, what environment was set, or whether the subprocess was producing any output. This made it impossible to distinguish between three hypothesized root causes (pipe deadlock, auth stall, or interactive prompt hang) without adding instrumentation and waiting for a third reproduction.

The core issue: `spawn_and_collect()` — the shared subprocess lifecycle function for all CLI builtin handlers — had zero observability between spawn and completion. A subprocess that hung for 600 seconds produced exactly zero log lines until the outer timeout fired.

## Guidance

### 1. Pre-spawn invocation log

Log the exact command argv, environment variable key set (names only, never values), and any conditional context before calling `spawn_and_collect`:

```rust
let env_keys_set: &[&str] = match ctx.github_token {
    Some(_) => &["GH_PROMPT_DISABLED", "GH_TOKEN"],
    None => &["GH_PROMPT_DISABLED"],
};
tracing::info!(
    tool = "run_gh",
    argv = ?&gh_args.args,
    repo = ?&gh_args.repo,
    env_keys_set = ?env_keys_set,
    has_github_token = ctx.github_token.is_some(),
    "run_gh invocation"
);
```

**Token redaction is structural:** the `env_keys_set` slice contains only `&'static str` key names. The actual token value is never read by the log statement.

### 2. Chunked reader with atomic byte counters

Replace atomic `read_to_end` with a chunked read loop that increments shared `Arc<AtomicUsize>` counters per chunk. This lets a separate progress task observe byte counts during the read itself:

```rust
async fn read_with_counter<R: AsyncRead + Unpin>(
    reader: R, max_bytes: usize, counter: Arc<AtomicUsize>,
) -> Vec<u8> {
    let mut take = reader.take(max_bytes as u64);
    let mut buf = Vec::with_capacity(max_bytes.min(8192));
    let mut chunk = [0u8; 256];
    loop {
        match take.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                counter.fetch_add(n, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!(bytes_read = counter.load(Ordering::Relaxed),
                    error = %e, "read_with_counter I/O error");
                break;
            }
        }
    }
    buf
}
```

### 3. Progress ticker at 30-second intervals

A `tokio::spawn`ed task that snapshots both counters every 30 seconds. Aborted on normal completion:

```rust
let progress_task = tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.tick().await; // discard immediate first tick
    loop {
        interval.tick().await;
        tracing::info!(tool = %name, stdout_bytes = ..., stderr_bytes = ...,
            elapsed_ms = ..., "spawn_and_collect progress");
    }
});
// ... tokio::join!(stdout_reader, stderr_reader, child.wait())
progress_task.abort();
```

### 4. Timeout log enrichment

Pre-compute a truncated input excerpt before `input` is moved into the execute call, then include it in timeout log lines:

```rust
const TOOL_TIMEOUT_INPUT_EXCERPT_LEN: usize = 200;
let input_excerpt: String = serde_json::to_string(&input)
    .unwrap_or_default()
    .chars()
    .take(TOOL_TIMEOUT_INPUT_EXCERPT_LEN)
    .collect();
// ... later, in the Err(_) timeout arm:
warn!(tool = %name, timeout_secs = timeout, input_excerpt = %input_excerpt,
    "tool execution timed out");
```

## Why This Matters

Without these diagnostics, a hung subprocess produces exactly one log line (the timeout warning) with no information about what command was running or whether it was producing output. On the next reproduction, the four surfaces discriminate between the three hypothesized causes:

- **Pipe deadlock:** progress ticker shows `stdout_bytes ≈ 10240` (MAX_OUTPUT_LEN ceiling) quickly, then no further change while reads are done but process hasn't exited
- **Auth/network stall:** progress ticker shows `stdout_bytes = 0` for the entire duration
- **Interactive prompt:** same as auth stall but identified by argv showing a subcommand that could prompt

The instrumentation applies to all CLI builtin handlers (run_gh, run_gws, gh_read, git ops) via the shared `spawn_and_collect` function. Worst-case log volume: 20 tool steps × (1 + ⌊elapsed/30⌋) lines per turn. Steady-state for fast tools: 1 completion log line per invocation.

## When to Apply

- When a CLI subprocess handler times out without actionable log output
- When adding new CLI builtin handlers that use `spawn_and_collect`
- When debugging subprocess lifecycle issues (pipe deadlock, zombie processes, auth stalls)

## Examples

**Before (no diagnostics):**
```
WARN builtin handler timed out  tool=run_gh  timeout_secs=600
```

**After (four diagnostic surfaces):**
```
INFO  run_gh invocation  tool=run_gh  argv=["run","view","12345","--log-failed"]  repo=Some("senara-solutions/mika")  env_keys_set=["GH_PROMPT_DISABLED","GH_TOKEN"]  has_github_token=true
INFO  spawn_and_collect progress  tool=gh  stdout_bytes=10240  stderr_bytes=0  elapsed_ms=30012
INFO  spawn_and_collect progress  tool=gh  stdout_bytes=10240  stderr_bytes=0  elapsed_ms=60025
...
WARN  builtin handler timed out  tool=run_gh  timeout_secs=600  input_excerpt={"command":["run","view","12345","--log-failed"],"repo":"senara-solutions/mika"}
```

The progress ticker pattern (`stdout_bytes=10240` stuck at the MAX_OUTPUT_LEN cap) strongly suggests pipe deadlock — the follow-up fix is to drop pipe handles before `child.wait()`.

## Related

- `docs/solutions/runtime-errors/builtin-handler-timeout-ignores-skill-config.md` — the timeout path itself (why run_gh inherits 600s from dev-pilot's skill_timeout)
- mika#900 — the diagnostic instrumentation ticket
- mika#515 — `MIKA_GITHUB_TOKEN→GH_TOKEN` re-injection in builtin handlers
