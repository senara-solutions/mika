---
title: "ToolCallSummary success flag contradicts non_zero_exit"
category: logic-errors
date: 2026-03-13
severity: medium
module: crates/mika-agent/src/agent.rs
issue: "#144"
tags: [tool-metadata, non-zero-exit, exec-handler, observability]
---

## Problem

`ToolCallSummary` reported `success: true` alongside `non_zero_exit: true` — a contradictory state. Any tool using exec handlers (shell-exec, github CLI, file-reader, marketplace skills) could produce this combination because exec handlers return `ToolOutput::success()` for non-zero exits (by design — grep uses exit code 1 for "no matches").

The `success` field in `ToolCallSummary` was derived solely from `!output.is_error`, ignoring the `non_zero_exit` heuristic.

## Root Cause

In `process_tool_calls()` (agent.rs:1017), `success` was set to `!output.is_error` without considering `non_zero_exit`. Since exec handlers deliberately return `ToolOutput::success()` for non-zero exits (the exit code is data for the agent to interpret, not an execution error), `is_error` was always `false` for subprocess results — even when the process exited non-zero.

Additionally, two reading functions (`format_tool_summary_block` and `format_step_exceeded_fallback`) checked `!success` before `non_zero_exit`, meaning after fixing the write side, non-zero exits would display as `[FAILED]` instead of `[NON-ZERO]`.

## Solution

Three-part fix in `crates/mika-agent/src/agent.rs`:

**1. Write side** — make `success` consider `non_zero_exit`:
```rust
// Before
success: !output.is_error,
// After
success: !output.is_error && !non_zero_exit,
```

**2. Read side** — reorder conditionals to check `non_zero_exit` first (preserves `[NON-ZERO]` vs `[FAILED]` distinction for both old and new metadata):
```rust
// Before: !success checked first, non_zero_exit unreachable for new data
let status = if !success { " [FAILED]" } else if non_zero_exit { " [NON-ZERO]" } else { "" };
// After: non_zero_exit checked first, works for both old and new data
let status = if non_zero_exit { " [NON-ZERO]" } else if !success { " [FAILED]" } else { "" };
```

Applied to both `format_tool_summary_block` (history context injection) and `format_step_exceeded_fallback` (max-steps fallback).

**3. Tests** — added backward-compat tests (old format: `success: true, non_zero_exit: true`) alongside new-format tests (`success: false, non_zero_exit: true`). Both assert `[NON-ZERO]` appears.

## Prevention

- When adding boolean flags to serialized structs, consider all consumers (write path + every read path) and ensure invariants are maintained across all of them.
- When a derived field (`success`) depends on multiple inputs (`is_error`, `non_zero_exit`), encode the full derivation at the construction site — don't leave it to readers to combine fields.
- Test backward compatibility with old serialized data whenever changing field semantics on persisted structs.

## Related

- `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md` — the original fix that introduced `non_zero_exit` and the `ToolOutput::success()` pattern for non-zero exits
- `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md` — metadata serialization pipeline
- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — dashboard consumption of `ToolCallSummary`
