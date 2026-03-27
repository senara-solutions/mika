---
title: "TUI callback path uses generic framing, skipping mika-qa delegation"
category: logic-errors
date: 2026-03-26
severity: high
tags: [callback, tui, self-dev, delegation, mika-qa, workflow-continuation]
issue: 269
modules: [mika-agent, mika-cli]
files:
  - crates/mika-agent/src/agent.rs
  - crates/mika-cli/src/commands/chat.rs
---

# TUI callback path uses generic framing, skipping mika-qa delegation

## Problem

When a `long_running:run_claude_pilot` callback was delivered in a TUI chat session, the mika-qa acceptance review was never triggered. The PR got created but had no QA review — the LLM summarized the callback result and stopped without calling `delegate_task("mika-qa", ...)`.

## Root Cause

Two callback framing functions existed with different behavior:

| Function | Used by | Workflow-aware? |
|----------|---------|-----------------|
| `build_callback_trigger_context()` | Silent agent (`SilentTrigger::Callback`) | Yes — detects claude-pilot label, injects mandatory mika-qa delegation instructions |
| `format_callback_framing()` | TUI chat callback handler | No — generic XML framing with "report what this result states" |

The TUI callback handler in `chat.rs` called `format_callback_framing()` directly, bypassing the workflow-aware routing in `build_callback_trigger_context()`.

This was introduced in #264, which added `build_callback_trigger_context()` for the silent path but didn't update the TUI path to use it.

## Solution

1. Made `build_callback_trigger_context()` public (was private `fn`).
2. Replaced `format_callback_framing()` with `build_callback_trigger_context()` in the TUI callback handler.

Both paths now get identical workflow-aware framing. `format_callback_framing()` is still called internally by `build_callback_trigger_context()` for the base XML framing.

## Pattern: Dual-path consistency

When the same logical operation (callback delivery) has two code paths (silent + interactive), ensure both paths use the same high-level function. The low-level function (`format_callback_framing`) should be an implementation detail, not a public API consumed by callers that need the full behavior.

## Prevention

- When adding workflow-specific behavior to one callback path, grep for all call sites of the base function to ensure the other path is also updated.
- The `build_callback_trigger_context` function is now the single entry point for all callback framing — future workflow routing changes only need to touch one function.
