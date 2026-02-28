---
title: "TUI Bugs: Think Display, Web Search Config Cascade, Clipboard Feedback"
category: logic-errors
tags:
  - tui
  - configuration
  - config-rs
  - web-search
  - clipboard
  - rust
symptoms:
  - Think level not visible in footer until explicitly set via /think
  - Web search says API key not configured even when set in config.toml
  - Ctrl+V image paste fails silently with no user feedback
root_cause:
  - Conditional rendering skipped default/off state in footer
  - web_search handler read env var directly, bypassing config-rs cascade
  - Clipboard error only logged via tracing::debug(), invisible in TUI
date: 2026-02-28
pr: 28
branch: feat/tui-slash-commands-and-web-search
---

# TUI Bugs: Think Display, Web Search Config Cascade, Clipboard Feedback

## Problem

After shipping the TUI slash commands PR (#28), user testing revealed three bugs:

1. **Think level invisible** — Footer showed nothing until `/think high` was run. Users didn't know the feature existed.
2. **Web search API key not found** — Setting `brave_api_key = "BSA..."` in `~/.mika/config.toml` had no effect. The handler only checked `std::env::var("MIKA_BRAVE_API_KEY")`.
3. **Silent clipboard failure** — Pressing Ctrl+V when no image was in the clipboard produced no visible feedback. Only a `tracing::debug!()` message was emitted.

## Investigation

### Bug 1: Think Display

In `crates/mika-cli/src/tui/ui.rs`, the footer used `if let Some(level) = app.thinking_level` to conditionally render the thinking indicator. When `thinking_level` was `None` (the default), nothing was rendered.

### Bug 2: Config Cascade

The root insight: **config-rs deserializes TOML + env vars into a Settings struct but does NOT call `std::env::set_var()`**. So `std::env::var("MIKA_BRAVE_API_KEY")` only works if the user sets the actual environment variable — it cannot read values from config.toml files.

The config cascade is:
1. `config/default.toml` (bundled)
2. `config/local.toml` (gitignored)
3. `~/.mika/config.toml` (user home)
4. `~/.mika/agents/<name>/config.toml` (per-agent)
5. `MIKA_*` env vars (highest priority)

All five layers are resolved into `Settings` at startup. Any code that reads env directly bypasses layers 1-4.

### Bug 3: Clipboard Feedback

In `crates/mika-cli/src/tui/input.rs`, the `ClipboardResult::Error` match arm only called `tracing::debug!()`. The user saw nothing in the TUI — the paste appeared to be silently ignored.

## Solution

### Bug 1: Always render thinking level

Replace conditional rendering with a `match` that handles both states:

```rust
// Always shown in footer
match app.thinking_level {
    Some((_, level)) => Span::styled(format!("think: {level}"), Style::default().fg(Color::Magenta)),
    None => Span::styled("think: off", Style::default().fg(Color::DarkGray)),
}
```

**File:** `crates/mika-cli/src/tui/ui.rs`

### Bug 2: Thread brave_api_key through config cascade

Added `brave_api_key: Option<String>` to `Settings` struct, then threaded through the dependency tree:

```
Settings → ToolContext → AgentParams / SilentAgentParams / TeamAgentParams
                       → AppState → server handlers
                       → ReminderScheduler
                       → TeamEngine
```

Handler reads from context instead of env:

```rust
let api_key = match ctx.brave_api_key {
    Some(key) if !key.trim().is_empty() => key.to_string(),
    _ => return ToolOutput::error("Brave Search API key not configured. Set brave_api_key in ~/.mika/config.toml or MIKA_BRAVE_API_KEY env var."),
};
```

**Files changed:** 16 (config.rs, tools/mod.rs, agent.rs, server/state.rs, server/mod.rs, server/handlers.rs, scheduler.rs, teams/engine.rs, chat.rs, ask.rs, test_utils.rs, send_message.rs, run_team.rs, builtin_handlers.rs)

### Bug 3: Show system message on clipboard error

```rust
ClipboardResult::Error(msg) => {
    tracing::debug!("clipboard image failed: {msg}");
    app.messages.push(ChatMessage {
        role: ChatRole::System,
        content: "Clipboard image not available. Use /attach <path> to attach an image file.".to_string(),
        rendered: None,
        channel: None,
    });
    return;
}
```

**File:** `crates/mika-cli/src/tui/input.rs`

### Additional hardening from code review

- **Shared HTTP client:** `LazyLock<reqwest::Client>` with 15s timeout, replacing per-call `Client::new()`
- **Response body limit:** 1MB cap on Brave API responses (Content-Length check + bytes limit)
- **Error sanitization:** Generic "Search request failed." instead of `format!("{e}")` (prevents network detail leakage)
- **Clipboard timeout:** 3s via thread + `mpsc::channel` pattern for xclip/wl-paste subprocess calls
- **Dead code removal:** Deleted `templates/skills/web-search/handlers/search.sh` after exec→builtin migration

## Prevention Strategies

### 1. Never read MIKA_* env vars directly

Always go through `Settings` or `ToolContext`. The config cascade is only resolved during config-rs deserialization. Direct `std::env::var` reads bypass config files entirely.

**Rule:** In code review, flag any `std::env::var("MIKA_")` as a blocker.

### 2. User-visible errors for user actions

Any error triggered by user input (Ctrl+V, slash command, button) must produce a `ChatMessage` with `role: System`. `tracing::debug!()` is invisible to users.

**Rule:** If user did something → they must see feedback.

### 3. Always render feature state, even defaults

"think: off" tells the user the feature exists. A blank footer tells them nothing.

**Rule:** If a feature has an off/default state, show it in the UI.

### 4. Sanitize external error messages

Never pass raw `reqwest::Error` to users — may contain internal URLs, IPs, or auth tokens.

### 5. Timeout all subprocess calls

Clipboard tools can hang if display server is unreachable. Always set a timeout (2-3s for interactive operations).

## Related

- PR #28: feat/tui-slash-commands-and-web-search
- `docs/configuration.md` — Config cascade documentation
- `docs/slash-commands.md` — Updated with /think, /model, /attach commands
