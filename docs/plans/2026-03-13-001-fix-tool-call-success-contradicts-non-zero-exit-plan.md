---
title: "fix: tool call success flag contradicts non_zero_exit"
type: fix
status: completed
date: 2026-03-13
---

# fix: tool call success flag contradicts non_zero_exit

## Overview

`ToolCallSummary.success` is set to `true` even when `non_zero_exit` is `true`, creating contradictory metadata. This affects **all tools using exec handlers** (shell-exec, github CLI, file-reader, and any marketplace skill with exec handlers) — not just `read_file`.

**Root cause:** In `process_tool_calls()` (`agent.rs:1017`), `success` was derived solely from `!output.is_error`. Exec handlers deliberately return `ToolOutput::success()` for non-zero exits (because tools like grep use exit code 1 for non-error conditions), so `is_error` is always `false` for subprocess results.

**GitHub Issue:** #144

## Proposed Solution

Three-part fix:
1. **Write side** — make `success` consider `non_zero_exit` (already done in working tree)
2. **Read side** — reorder conditionals in both formatting functions to check `non_zero_exit` before `!success`, preserving the `[NON-ZERO]` vs `[FAILED]` distinction
3. **Tests** — update existing tests and add backward-compatibility coverage

## System-Wide Impact

- **Affected code paths:** `process_tool_calls` (write), `format_tool_summary_block` (history context injection), `format_step_exceeded_fallback` (max-steps fallback). All in `crates/mika-agent/src/agent.rs`.
- **Affected tools:** All exec-handler-based tools — shell-exec, github CLI (`builtin_handlers.rs:386`), file-reader, and any skill with `handler = "exec"`.
- **Silent/heartbeat mode:** Uses same `process_tool_calls` — fix propagates automatically.
- **Team engine:** Delegates to same tool loop — fix propagates automatically.
- **Backward compatibility:** Old DB rows may have `success: true, non_zero_exit: true`. Reordering the read-side conditionals handles both old and new data correctly.
- **Dashboard:** Frontend `parseToolCalls()` reads `success` and `non_zero_exit` independently — no dashboard changes needed.

## Acceptance Criteria

- [x] `success` is `false` whenever `non_zero_exit` is `true` in newly-written metadata (`agent.rs:1017`)
- [x] `format_tool_summary_block` shows `[NON-ZERO]` (not `[FAILED]`) for non-zero exit entries — both old format (`success: true, non_zero_exit: true`) and new format (`success: false, non_zero_exit: true`)
- [x] `format_step_exceeded_fallback` shows `"non-zero exit"` (not `"failed"`) for non-zero exit entries — both formats
- [x] Doc comment on `non_zero_exit` field updated to reflect new semantics
- [x] Tests cover both old-format and new-format metadata
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## MVP

### 1. Write side — `process_tool_calls` (`agent.rs:1017`)

Already done in working tree:

```rust
success: !output.is_error && !non_zero_exit,
```

### 2. Read side — `format_tool_summary_block` (`agent.rs:274-280`)

Reorder to check `non_zero_exit` first:

```rust
let status = if non_zero_exit {
    " [NON-ZERO]"
} else if !success {
    " [FAILED]"
} else {
    ""
};
```

### 3. Read side — `format_step_exceeded_fallback` (`agent.rs:305-311`)

Same reorder:

```rust
let status = if s.non_zero_exit {
    "non-zero exit"
} else if !s.success {
    "failed"
} else {
    "done"
};
```

### 4. Tests (`agent.rs:~2292, ~2321`)

Update existing test JSON to use new-format data (`success: false, non_zero_exit: true`) and add backward-compat tests with old-format data (`success: true, non_zero_exit: true`). Both should assert `[NON-ZERO]` appears.

## Sources

- Related issue: #144
- Institutional learning: `docs/solutions/logic-errors/exec-handler-stdout-discarded-on-nonzero-exit.md`
- Institutional learning: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md`
