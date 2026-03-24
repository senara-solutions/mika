---
title: "fix: callback result truncation causes agent timeout on large claude-pilot runs"
type: fix
status: completed
date: 2026-03-24
issue: 259
---

# fix: callback result truncation causes agent timeout on large runs

## Problem

After large claude-pilot runs (~90KB callback results), the silent agent times out during prompt assembly. The full result is injected into the system prompt via `format_callback_framing()`, and the serialization/setup phase consumes the entire 5-minute `AGENT_TOTAL_TIMEOUT_SECS` (300s). The agent never reaches the LLM API call — zero tool calls, work item left unprocessed.

**Evidence:** Task `3a27a03f` (mika#257): callback delivered at 14:00:30, timed out at 14:05:30.

## Root Cause

1. `format_callback_framing()` (agent.rs:67) injects the raw `result` string with no size cap
2. Server handler accepts up to 100KB (`handlers.rs:327`), `complete_task` tool accepts up to 100KB
3. The `run_agent_inner()` is wrapped in a 300s timeout (agent.rs:707)
4. A 90KB system prompt overwhelms serialization/setup within the timeout window

## Fix

Add a **Rust-side safety cap** in `format_callback_framing()`: truncate the `result` string to 10KB before injection. This protects against any handler sending oversized results, regardless of source.

### Changes

#### `crates/mika-agent/src/agent.rs`

1. Add constant: `const CALLBACK_RESULT_MAX_BYTES: usize = 10_240;`
2. In `format_callback_framing()`, truncate `result` before injection using the existing `truncate_summary()` pattern (UTF-8-safe byte boundary truncation), but with a callback-specific suffix: `"\n...\n[truncated — full result available in task logs]"`
3. Add tests for truncation behavior (short result unchanged, long result truncated, UTF-8 boundary safety)

### Out of Scope (companion change)

The `mika-skills/claude-pilot/handlers/run.sh` handler should also reduce its log tail from ~90KB to 10KB. That change belongs in the `mika-skills` repo and should be tracked separately.

## Acceptance Criteria

- [x] `format_callback_framing()` truncates results > 10KB with a clear suffix message
- [x] Truncation is UTF-8-safe (no panics on multi-byte input at the boundary)
- [x] Existing tests pass unchanged (they use short results)
- [x] New test: result > 10KB is truncated with suffix
- [x] New test: result <= 10KB passes through unchanged
- [x] New test: UTF-8 multi-byte at boundary is safe
- [x] `cargo test` passes, `cargo clippy` clean

## Context

- The `truncate_summary()` helper already exists in agent.rs (line 191) for UTF-8-safe byte truncation
- The 100KB cap at the API/tool layer remains unchanged — truncation is only at the prompt injection point
- Full callback results are always available in task logs for debugging
