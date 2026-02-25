---
title: "Fix TUI bugs: empty response, no history, no logs, no input wrap"
type: fix
status: completed
date: 2026-02-25
---

# Fix TUI Bugs: Empty Response, No History, No Logs, No Input Wrap

## Overview

Four user-facing bugs in the Mika CLI TUI that degrade the chat experience:

1. **Empty response display** — After a few turns, TUI shows "Mika: " with no text
2. **No conversation persistence** — Restarting loses all history
3. **No file logging** — `.mika/agents/main/logs` is empty
4. **No input wrapping** — Text scrolls left instead of wrapping at window edge

## Bug 1: Empty "Mika: " Response

### Root Cause

`agent.rs:189-195` returns an empty string when the model responds with tool-use blocks only (no text blocks) then `EndTurn`. The TUI's `app.rs:220-223` starts progressive reveal on this empty string, rendering only the "Mika: " prefix.

```rust
// agent.rs:189-195
StopReason::EndTurn | StopReason::MaxTokens => {
    let text = response.text();  // "" when only tool_use blocks
    if !text.is_empty() {
        db.save_message("assistant", &text, channel_type).await?;
    }
    return Ok(text);  // returns ""
}
```

### Fix

Guard in `app.rs` `tick()` — skip display when content is empty:

```rust
// app.rs tick() — guard empty responses
if response.content.is_empty() {
    self.status = AgentStatus::Idle;
    self.needs_redraw = true;
} else {
    self.pending_response = Some(response.content);
    self.reveal_index = 0;
    self.status = AgentStatus::Responding(0);
}
```

### Files
- [x] `crates/mika-cli/src/tui/app.rs` — Add empty content guard in `tick()`

## Bug 2: No Conversation Persistence

### Root Cause

Two issues:
1. `chat.rs:44` generates a new `Uuid::new_v4()` every launch — no session continuity
2. `App::new()` at `app.rs:123` initializes `messages: Vec::new()` — never loads history from DB

The agent does load messages for context (`agent.rs:142` calls `db.load_recent_messages(20, None)`), but the TUI never displays them.

### Fix

1. Load recent messages from DB at startup and populate `app.messages`
2. Use the most recent session_id from DB (or create new if none exists)
3. Filter to `cli` channel type to avoid showing Telegram messages

```rust
// After App::new(), load history
let recent = ctx.async_db.load_recent_messages(20, Some("cli")).await?;
for msg in recent {
    app.messages.push(ChatMessage {
        role: match msg.role.as_str() {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => continue,
        },
        content: msg.content,
        rendered: None,
    });
}
```

For session_id, query the most recent session from the DB or fall back to a new UUID.

### Files
- [x] `crates/mika-cli/src/commands/chat.rs` — Load history after App::new(), session recovery
- [x] `crates/mika-cli/src/tui/app.rs` — May need method to bulk-load messages
- [x] `crates/mika-agent/src/db.rs` — May need `get_last_session_id()` query

## Bug 3: No File Logging

### Root Cause

`logging.rs` only writes to stderr via `fmt::layer().pretty()`. No file appender configured. The `logs/` directory is never created or written to.

```rust
// logging.rs — current implementation (stderr only)
pub fn init_pretty(default_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().pretty())
        .init();
}
```

### Fix

Add `tracing-appender` dependency. Create a daily-rotating file appender writing to `{home_dir}/logs/`. Return `WorkerGuard` from init functions — guard MUST be held alive in `main()` or logging silently stops.

```rust
pub fn init_pretty(default_level: &str, log_dir: &Path) -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily(log_dir, "mika.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().pretty()) // stderr
        .with(fmt::layer().json().with_writer(non_blocking)) // file
        .init();

    guard
}
```

Key concern: `WorkerGuard` lifetime must be managed — store in `main()` scope. Initialization ordering matters since `init_pretty` is called before `init_for_agent`.

### Files
- [x] `Cargo.toml` (workspace) — Add `tracing-appender` dependency
- [x] `crates/mika-common/Cargo.toml` — Add `tracing-appender` dep
- [x] `crates/mika-common/src/logging.rs` — Add file appender with daily rotation
- [x] `crates/mika-cli/src/main.rs` — Hold WorkerGuard, pass log_dir
- [x] `crates/mika-cli/src/init.rs` — May need to create logs directory

## Bug 4: Input Text Doesn't Wrap

### Root Cause

`tui-textarea` v0.7 does horizontal scrolling by default — no soft-wrap support. The input area is fixed at `Constraint::Length(3)` in `ui.rs:14`. When text exceeds the visible width, it scrolls left instead of wrapping.

### Fix

Use a dynamic input height approach. Calculate required lines based on content width and grow the input area. This is a partial fix since `tui-textarea` itself doesn't soft-wrap, but we can:

1. Make the input area height dynamic based on content length
2. Set a reasonable max height (e.g., 6 lines)
3. The textarea will at least show more content as lines grow

```rust
// ui.rs — dynamic input height
let input_text_len = app.textarea.lines()[0].len();
let available_width = f.area().width.saturating_sub(4) as usize; // -4 for borders/prompt
let input_lines = if available_width > 0 {
    ((input_text_len / available_width) + 1).min(6) as u16
} else {
    1
};
let input_height = input_lines + 2; // +2 for padding

let chunks = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(5),
    Constraint::Length(input_height),
    Constraint::Length(1),
])
.split(f.area());
```

### Files
- [x] `crates/mika-cli/src/tui/ui.rs` — Dynamic input height calculation

## Acceptance Criteria

- [x] Empty agent responses (tool-use only) don't show bare "Mika: " prefix
- [x] Restarting `mika` shows recent conversation history
- [x] Log files appear in `~/.mika/agents/{name}/logs/` with daily rotation
- [x] Long input text grows the input area vertically (up to 6 lines)
- [x] All existing tests pass (`cargo test`)
- [x] No regressions in TUI rendering

## Implementation Order

1. Bug 1 (empty response) — smallest, most impactful
2. Bug 3 (logging) — enables debugging for other fixes
3. Bug 2 (history) — requires DB queries, most complex
4. Bug 4 (input wrap) — UI-only, independent

## References

- `crates/mika-cli/src/tui/app.rs` — App struct, tick() method
- `crates/mika-cli/src/commands/chat.rs` — spawn_agent_worker, session_id
- `crates/mika-cli/src/tui/ui.rs` — draw functions, layout
- `crates/mika-common/src/logging.rs` — logging initialization
- `crates/mika-agent/src/agent.rs` — agent loop, empty text return
- `crates/mika-agent/src/db.rs` — load_recent_messages
- Related: `docs/solutions/runtime-errors/claude-api-error-message-formatting.md` — established `{e}` vs `{e:#}` pattern
