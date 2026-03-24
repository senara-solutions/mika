---
title: "Callback processing race steals TUI notifications and skips mika-qa review"
category: logic-errors
date: 2026-03-24
severity: high
tags: [task-engine, callback, race-condition, tui, silent-agent, cli-mode, self-dev]
issue: 264
modules: [mika-agent, mika-cli]
files:
  - crates/mika-agent/src/task_engine/dispatcher.rs
  - crates/mika-agent/src/task_engine/engine.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-cli/src/commands/chat.rs
  - crates/mika-agent/src/server/mod.rs
---

# Callback processing race steals TUI notifications and skips mika-qa review

## Problem

Two independent systems competed for callback tasks in CLI mode:

- **TUI `poll_callback_tasks()`** — polls every ~5s, session-scoped, claims atomically before processing, runs `run_agent()` with full conversation history.
- **Engine `dispatch_undelivered_callbacks()`** — scans every 60s, queries ALL callbacks, runs `run_silent_agent()` in a new context-free `callback-{uuid}` session.

When the engine won the race, the callback was processed in a silent turn that the user never saw. The generic "analyze and notify" trigger instruction caused the agent to skip mika-qa delegation and mark work items complete prematurely.

## Root Cause

1. **Race condition:** Both TUI polling and engine dispatch were active in CLI mode. The engine's `dispatch_undelivered_callbacks()` ran on every 60-tick DB scan, finding callbacks that the TUI hadn't yet claimed. The engine claimed them after processing (not before), while the TUI claimed them atomically before processing — but the engine could complete its silent turn before the TUI's next poll.

2. **Context-free callback processing:** The engine's `dispatch_resume_agent()` created a new `callback-{uuid}` session with no conversation history. The generic trigger instruction ("Analyze the data and use send_message to notify the user") directed the agent to summarize rather than continue the self-dev workflow. The self-dev skill prompt was injected (always_on), but without conversation history the agent followed the simpler instruction.

## Solution

### Fix 1: `cli_mode` guard on `TaskDispatcher`

Added `pub cli_mode: bool` to `TaskDispatcher`. In CLI mode (`cli_mode: true`), the engine skips `dispatch_undelivered_callbacks()` entirely. The TUI's `poll_callback_tasks()` becomes the sole callback consumer.

```rust
// crates/mika-agent/src/task_engine/engine.rs — tick()
if !self.dispatcher.cli_mode {
    self.dispatch_undelivered_callbacks().await;
}
```

Construction sites:
- `chat.rs`: `cli_mode: true` (TUI handles callbacks)
- `server/mod.rs`: `cli_mode: false` (engine handles callbacks)

### Fix 2: Workflow-aware callback trigger

Extracted `build_callback_trigger_context()` from inline code in `run_silent_inner()`. Routes claude-pilot callbacks to self-dev workflow continuation instructions based on exact label match (`long_running:run_claude_pilot`):

- **Success:** Delegate to mika-qa, manage work items based on verdict
- **Failure:** Escalation — notify user, do not retry, leave work item status unchanged
- **Other labels:** Generic "analyze and notify" (unchanged)

## Prevention

- **Mode-aware dispatch:** When two systems can consume the same resource (callback tasks), use a mode flag to ensure only one consumer is active per deployment mode. The TUI path is superior in CLI mode (richer context); the engine path is necessary in server mode (no TUI).
- **Exact label matching for workflow-specific behavior:** Use `==` (not `.contains()`) to prevent false positives on similar label names. The label format `long_running:{tool_name}` is stable (set by `executor.rs`).
- **Test the negative case:** The `test_cli_mode_skips_callback_dispatch` test verifies the guard by inserting a completed callback, running 61 ticks, and asserting the task remains in `completed` status (not `delivered`).

## Related

- [callback-tui-delivery-polling.md](../architecture-patterns/callback-tui-delivery-polling.md) — TUI polling architecture
- [callback-task-loop-prevention.md](../architecture-patterns/callback-task-loop-prevention.md) — Four loop-prevention layers
- [callback-resume-agent-lifecycle.md](../architecture/callback-resume-agent-lifecycle.md) — Full callback lifecycle
- [failed-callback-tasks-silently-dropped.md](./failed-callback-tasks-silently-dropped.md) — Previous callback status filter bug
- [callback-result-too-large-causes-agent-timeout.md](../runtime-errors/callback-result-too-large-causes-agent-timeout.md) — Companion truncation fix
