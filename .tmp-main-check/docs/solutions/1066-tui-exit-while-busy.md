---
module: mika-cli
tags: [tui, input, autonomous-agents, tmux]
problem_type: bug
category: tui-input
---

# TUI silently drops Enter while busy — /exit unusable on autonomous agents

## Problem

The TUI Enter handler in `crates/mika-cli/src/tui/input.rs` gated ALL input submission on `AgentStatus::Idle`. When the agent was `Thinking` or `Responding`, pressing Enter was silently ignored. This meant `/exit`, `/quit`, and `/q` were unreachable while the agent was busy.

**Impact:** Autonomous agents (Mika Prime) spend virtually all their time in non-Idle states. The `mika-platform-agents-tmux stop` script sends `/exit` + Enter via `tmux send-keys`, but this was silently dropped, leaving the Mika Prime session running while all other agents stopped cleanly.

## Root cause

Two layers of Idle gating:

1. **`handle_key_normal()`** — the Enter key handler checked `app.status == AgentStatus::Idle` before calling `app.send_message()`.
2. **`handle_enter_completion()`** — the autocomplete Enter handler also checked `app.status == AgentStatus::Idle` before executing no-args commands like `/exit`.

Since `tmux send-keys` delivers characters as individual key events, the autocomplete popup would be visible when Enter arrived, routing through the second path.

## Solution

Added a quit-command fast-path in `handle_key()` (the top-level key dispatcher), **before** the autocomplete/normal dispatch branch:

```rust
if key.code == KeyCode::Enter && !key.modifiers.intersects(SHIFT | ALT) {
    let input_text = app.input_text();
    if matches!(input_text.trim(), "/exit" | "/quit" | "/q") {
        app.should_quit = true;
        return;
    }
}
```

This catches quit commands regardless of:
- Agent status (Idle, Thinking, Responding)
- Autocomplete visibility
- Leading/trailing whitespace

The existing Idle gate for regular messages is preserved — only the three always-safe quit aliases bypass it.

## Why this location

The fast-path must be in `handle_key()`, not `handle_key_normal()`, because `tmux send-keys` delivers individual keystrokes that trigger autocomplete. By the time Enter arrives, the autocomplete popup intercepts it. Placing the check before the autocomplete/normal branch covers both paths.

## Precedent

`should_quit` is already set from non-Idle context by the Ctrl+C handler (same file, `SelectionState::None` branch). No new state transitions or async interactions introduced.

## Test coverage

Five tests added in `crates/mika-cli/src/tui/input.rs`:
- `/exit` while `Thinking` → `should_quit == true`
- `/quit` while `Responding` → `should_quit == true`
- `/q` while `Thinking` → `should_quit == true`
- Regular message while `Thinking` → no send, `should_quit == false`
- `/exit` while `Idle` → `should_quit == true`
