---
type: fix
issue: 1066
module: mika-cli
tags: [tui, input, autonomous-agents]
---

# Plan: TUI silently drops Enter while busy — /exit unusable on autonomous agents

## Problem

The TUI input handler (`crates/mika-cli/src/tui/input.rs:647-656`) gates ALL Enter presses on `app.status == AgentStatus::Idle`. This means `/exit`, `/quit`, and `/q` — which are always-safe terminal commands — cannot be submitted while the agent is in `Thinking` or `Responding` state. Since Mika Prime spends virtually all its time in one of those states, there is no way to gracefully stop it via TUI input or `tmux send-keys`.

**Primary consumer:** The `scripts/mika-platform-agents-tmux stop` script, which sends `/exit` + Enter via `tmux send-keys` to every running agent session. Interactive users can close the terminal, but programmatic callers have no alternative when the agent is busy.

## Pinned Source

### Enter handler (`crates/mika-cli/src/tui/input.rs:644-656`)

```rust
// Enter sends message or executes slash command (only when idle and not shift/alt-held)
// Shift+Enter and Alt+Enter insert a newline instead (Alt+Enter as universal fallback
// for terminals where Shift+Enter is indistinguishable from Enter)
if key.code == KeyCode::Enter
    && !key
        .modifiers
        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
{
    if app.status == AgentStatus::Idle {
        app.send_message();
    }
    return;
}
```

The Idle gate is the first (and only) thing after the Enter key match. There is no preamble logic between the key match and the gate. `app.input_text()` is not called here — it's called inside `send_message()`. The fast-path can call it before the gate.

### `/exit` dispatch (`crates/mika-cli/src/tui/commands/handlers.rs:37-39`)

```rust
"exit" | "quit" | "q" => {
    app.should_quit = true;
    None
}
```

This is inside the async `dispatch()` function. The fast-path mimics this exact effect (setting `should_quit = true`) without routing through async dispatch.

### Ctrl+C handler (`crates/mika-cli/src/tui/input.rs:274-280`)

```rust
SelectionState::None => {
    let input_text = app.input_text();
    if !input_text.is_empty() {
        copy_text_to_clipboard(&input_text);
    } else {
        app.should_quit = true;
    }
}
```

Confirms `should_quit` is already set from the synchronous input handler without any status check. The flag is safe to set from any agent state — the main loop checks it on each iteration and performs graceful shutdown via `AgentRequest::Quit`.

## Solution

Add a fast-path check in the Enter handler: before the Idle gate, check if the trimmed input is a quit command. If so, set `app.should_quit = true` directly and return. Non-quit slash commands and regular messages remain gated behind `AgentStatus::Idle`.

**Design decisions (resolved):**
1. **Inline match, not extracted function.** Three fixed strings with no other callers — extracting to a function adds indirection for zero polymorphism benefit.
2. **Input buffer is not cleared.** `should_quit = true` triggers shutdown on the next main loop iteration; no subsequent code reads the buffer.
3. **Case-sensitive match.** The existing slash command parser (`parse_command()`) is case-sensitive; `/EXIT` is not recognized anywhere. tmux scripts send lowercase `/exit`.

## Changes

### 1. `crates/mika-cli/src/tui/input.rs` — Enter key handler (L647-656)

**Before:**
```rust
if key.code == KeyCode::Enter
    && !key
        .modifiers
        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
{
    if app.status == AgentStatus::Idle {
        app.send_message();
    }
    return;
}
```

**After (verbatim insertion):**
```rust
if key.code == KeyCode::Enter
    && !key
        .modifiers
        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
{
    // Always-safe quit commands bypass the Idle gate.
    // Critical for autonomous agents that are rarely Idle — without this,
    // `tmux send-keys "/exit" Enter` is silently dropped while busy.
    let input_text = app.input_text();
    if matches!(input_text.trim(), "/exit" | "/quit" | "/q") {
        app.should_quit = true;
        return;
    }
    if app.status == AgentStatus::Idle {
        app.send_message();
    }
    return;
}
```

`input_text()` returns an owned `String` from the textarea widget — no lifetime concern. The `trim()` handles leading/trailing whitespace from tmux send-keys. The match is exhaustive against the three quit aliases defined in `handlers.rs:37`.

### 2. `crates/mika-cli/src/tui/input.rs` — Unit test

Add a test that sets `app.status = AgentStatus::Thinking`, puts `/exit` in the input buffer, simulates Enter, and asserts `app.should_quit == true`.

## Out of scope (per ticket)

- Force-quit semantics for Ctrl+C with non-empty input
- Script-side fallback (`tmux kill-session` after timeout)
- Allowing other slash commands while busy (keep scoped to terminal commands)

## Risks

- **None material.** Single added conditional before the existing gate. No new state transitions, no async, no channel communication. The `should_quit` flag is already set from non-Idle context by the Ctrl+C handler (pinned above).

## Test plan

1. `cargo test -p mika-cli` — existing tests pass
2. New unit test: Enter on `/exit` while Thinking → `should_quit == true`
3. Manual: start `mika`, trigger a long response, type `/exit` + Enter while Responding → session exits
4. Manual: `tmux send-keys -t mika "/exit" Enter` while Mika Prime is Thinking → session exits
