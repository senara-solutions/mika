---
title: "Long-running handler silently falls through to sync exec path"
category: logic-errors
date: 2026-04-12
tags: [skills, executor, long-running, callback, silent-mode]
issue: "#537"
---

# Long-running handler silently falls through to sync exec path

## Problem

When `execute_skill_tool` dispatches a skill tool with `handler.long_running = true` but `long_running_ctx` is `None` (callback turns, silent mode, CLI test), the function silently falls through to the synchronous `execute_exec` path. The sync path does not inject `__mika_task_id` / `__mika_agent` env vars, so handlers expecting long-running metadata crash with a cryptic exit-code error (`no __mika_task_id in input`) that looks like a handler bug, not an engine dispatch bug.

## Root Cause

The `if let` guard at `executor.rs:112-128` uses a compound pattern matching **both** `long_running: true` AND `Some(ctx) = long_running_ctx`. When `long_running_ctx` is `None`, the entire block is skipped and control falls through to the timeout-wrapped `execute_inner` -> `execute_exec` — which runs the handler in sync mode without the long-running metadata injection that only happens in `execute_long_running`.

The `lr_ctx = None` for callback turns was introduced intentionally in commit `04ae084c` to prevent recursion, but the implementation chose "silently fall through" instead of "refuse with a clear error".

## Solution

Added an explicit guard in `execute_skill_tool` (after the existing long-running dispatch block) that checks if the handler is `ToolHandler::Exec { long_running: true, .. }` and `long_running_ctx` is `None`. When both conditions are true, it returns `ToolOutput::error(...)` with a message that:

1. Names the tool
2. Explains that long-running tools cannot run in the current context
3. Lists the contexts where this restriction applies (callback turn, silent mode, CLI test)

```rust
if matches!(
    &skill_tool.handler,
    ToolHandler::Exec { long_running: true, .. }
) && long_running_ctx.is_none()
{
    return ToolOutput::error(format!(
        "Tool '{}' is declared long_running but cannot run in the current context \
         (callback turn, silent mode, or CLI test). Long-running tools require a \
         conversation-mode turn with an active task engine.",
        skill_tool.definition.name
    ));
}
```

**Key file:** `crates/mika-agent/src/skills/executor.rs`

## Prevention

When adding compound pattern guards that match on both a data variant AND an `Option`, always add an explicit else-branch for the `None` case rather than relying on fall-through behavior. The fall-through path should be reserved for genuinely compatible alternatives, not for incompatible dispatch modes that will fail downstream.

**Pattern to follow:**
```rust
// Match on (variant, Some(ctx)) -> dispatch
if let Variant { flag: true } = &item && let Some(ctx) = optional_ctx {
    return dispatch_with_ctx(ctx);
}
// Explicit guard for (variant, None) -> refuse
if matches!(&item, Variant { flag: true, .. }) && optional_ctx.is_none() {
    return error("cannot dispatch without context");
}
// Fall through for non-matching variants
```
