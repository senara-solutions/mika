---
title: "Remove /dashboard slash command, dispatch footer buttons via pending_dashboard_action"
category: ui-bugs
date: 2026-03-19
tags: [tui, ratatui, dashboard, footer, async, slash-command, terminal-corruption]
modules: [mika-cli, tui, commands, dashboard]
severity: medium
---

## Problem

The TUI had three ways to control the embedded dashboard: the `/dashboard` slash command, footer `[start]`/`[stop]` buttons, and footer `[open]` button. This was redundant, and two of the three paths had bugs:

1. **`[open]` button** called `open_dashboard_in_browser()` directly from the sync mouse handler. That function used `println!` which wrote to stdout, corrupting the ratatui terminal (raw mode).
2. **`[start]`/`[stop]` buttons** dispatched via `pending_command = Some("/dashboard start")` to the slash command handler. Removing the `/dashboard` command would break these buttons.

## Root Cause

The `[open]` button bypassed the async tick loop entirely, calling a function that assumed a normal terminal (not raw mode). The `[start]`/`[stop]` buttons were coupled to the slash command handler via string-based dispatch — a fragile indirection that would break if the command was removed.

## Solution

### 1. Remove `/dashboard` slash command entirely

Removed from three files:
- `commands/mod.rs` — `SlashCommand` entry from `COMMANDS` array
- `commands/handlers.rs` — `handle_dashboard()` function and match arm, plus `TEAM_MODE_ALLOWED_COMMANDS` entry
- `commands/completers.rs` — `complete_dashboard()` function

### 2. Add typed `DashboardAction` enum and `pending_dashboard_action` field

```rust
// crates/mika-cli/src/tui/app.rs
pub enum DashboardAction {
    Start,
    Stop,
    Open,
}

pub struct App<'a> {
    // ...
    pub pending_dashboard_action: Option<DashboardAction>,
}
```

This follows the same sync-to-async bridging pattern as the existing `pending_command: Option<String>`, but with compile-time safety via a typed enum.

### 3. Process in `tick()` (async context)

The `tick()` method handles `pending_dashboard_action` right after `pending_command`:
- Start/Stop: HTTP POST to mika-server toggle endpoints, update `dashboard_running` state
- Open: call `open_dashboard_in_browser()` (safe here because output goes to `ChatMessage`, not stdout)

### 4. Fix `open_dashboard_in_browser()` to return `String`

Changed from `println!` to returning a `String`. The CLI `open()` wrapper calls `println!("{}", open_dashboard_in_browser())`. The TUI tick handler puts the return value into a `ChatMessage` with `ChatRole::Command`.

### 5. Update footer click handlers

```rust
// crates/mika-cli/src/tui/input.rs — sync mouse handler
app.pending_dashboard_action = Some(DashboardAction::Start); // or Stop, Open
```

## Key Pattern: Sync-to-Async Bridge in ratatui TUI

Mouse handlers in ratatui are synchronous (called from the event loop). Any async work (HTTP calls, DB queries) must be deferred to the async `tick()` method. The pattern:

1. Sync handler sets an `Option<T>` field on `App`
2. Async `tick()` calls `.take()` and processes the action
3. Result displayed as `ChatMessage` with `ChatRole::Command`

This is the same pattern used by `pending_command` for slash commands and `pending_switch` for agent switching. Prefer typed enums over strings for new dispatch paths.

## Prevention

- **Never call `println!` from TUI code.** In ratatui raw mode, stdout writes corrupt the terminal. All user-visible output must go through `ChatMessage` or ratatui rendering.
- **Prefer typed enums over string dispatch.** `pending_dashboard_action: Option<DashboardAction>` catches missing match arms at compile time, unlike `pending_command: Option<String>`.
- **When removing a slash command, check all three files:** `mod.rs` (definition), `handlers.rs` (handler + team mode allowlist), `completers.rs` (completer).
