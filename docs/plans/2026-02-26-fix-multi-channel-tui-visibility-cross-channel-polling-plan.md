---
title: "fix: Multi-channel TUI visibility, cross-channel polling, and Telegram config"
type: fix
status: completed
date: 2026-02-26
---

# fix: Multi-Channel TUI Visibility, Cross-Channel Polling, and Telegram Config

## Overview

After sending a Telegram message to the agent (via mika-server), three problems emerge:
1. **TUI only loads CLI messages on startup** -- Telegram messages are invisible
2. **Messages received by mika-server while TUI is open don't appear** in real-time
3. **No way to configure Telegram chat_id from the CLI** -- only set via server handler

The agent loop already loads all channels (`load_recent_messages(20, None)`) so Claude sees everything, but the TUI is blind to non-CLI messages.

## Problem Statement

The TUI chat interface (`crates/mika-cli/src/commands/chat.rs:181`) hardcodes `Some(vec!["cli".to_string()])` as the channel filter, discarding Telegram messages from the display. The `ChatMessage` struct (`crates/mika-cli/src/tui/app.rs:28-33`) has no `channel` field, and the rendering code (`crates/mika-cli/src/tui/ui.rs:80-91`) renders all user messages as "You:" with no channel distinction. There is no polling mechanism to detect new messages from other channels, and no `/config set` command to configure Telegram integration from the CLI.

## Technical Approach

### Design Decisions

- **WAL mode already enabled** (`db.rs:65`) -- concurrent CLI reader + server writer works
- **Poll only non-CLI channels** -- CLI messages are already in the TUI via the agent response pipeline; polling `["telegram"]` avoids duplication
- **Watermark approach** -- track `last_seen_msg_id` to avoid re-fetching; update after each agent turn and after polling
- **Config allowlist** -- only `chat_id` and `timezone` are settable, preventing arbitrary config injection
- **Queue polled messages during agent processing** -- insert only when `AgentStatus::Idle` to avoid visual confusion mid-response
- **Preserve scroll position on polled messages** -- only auto-scroll when user is at bottom (`scroll_offset == 0`)
- **Channel prefix on ALL non-CLI messages** -- both user and assistant roles get `[telegram]` prefix for clarity
- **`Option<String>` for channel field** -- defaults to `None` for CLI-originated messages, avoiding breaking all existing `ChatMessage` constructors

### From Institutional Learnings

- **Scroll calculation** is already correct (visual row wrapping via `Line::width()`), no changes needed
- **Channel awareness** already in prompt via `write_channel_section()` with `VALID_CHANNELS` allowlist
- **Async DB pattern**: clone string params, use `self.with_db(move |db| ...)`, return `Result<T>`
- **TUI + stderr rule**: polling must not introduce logging that corrupts the TUI

## Implementation

### Phase 1: Show Telegram Messages in TUI History

#### 1.1 Add `channel` field to `ChatMessage`

**File:** `crates/mika-cli/src/tui/app.rs:28-33`

```rust
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub rendered: Option<Vec<Line<'static>>>,
    pub channel: Option<String>, // None = CLI, Some("telegram") = Telegram
}
```

- Add `channel: None` to all existing `ChatMessage` constructors in `app.rs` (~7 sites: lines 216, 249, 268, 278, 287, 304, 340)
- Add `channel: None` to constructors in `handlers.rs` (system messages from command handlers)

#### 1.2 Change history filter to include Telegram

**File:** `crates/mika-cli/src/commands/chat.rs:181`

- [ ] Change `Some(vec!["cli".to_string()])` to `Some(vec!["cli".to_string(), "telegram".to_string()])`
- [ ] Set `channel` field on loaded history messages:

```rust
for msg in history {
    let role = match msg.role.as_str() {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        _ => continue,
    };
    let channel = if msg.channel_type == "cli" {
        None
    } else {
        Some(msg.channel_type.clone())
    };
    app.messages.push(ChatMessage {
        role,
        content: msg.content,
        rendered: None,
        channel,
    });
}
```

#### 1.3 Render channel prefix for non-CLI messages

**File:** `crates/mika-cli/src/tui/ui.rs:80-91`

- [ ] In `ChatRole::User` rendering, prepend `[telegram] ` span in yellow when `msg.channel.as_deref() == Some("telegram")`
- [ ] In `ChatRole::Assistant` rendering, prepend `[telegram] ` span in yellow when `msg.channel.as_deref() == Some("telegram")`
- [ ] Use a helper to avoid duplication:

```rust
fn channel_prefix_span(channel: &Option<String>) -> Option<Span<'static>> {
    match channel.as_deref() {
        Some("telegram") => Some(Span::styled("[telegram] ", Style::default().fg(Color::Yellow))),
        Some(ch) => Some(Span::styled(format!("[{ch}] "), Style::default().fg(Color::Yellow))),
        None => None,
    }
}
```

### Phase 2: `/config set` Command for Telegram Setup

#### 2.1 Add `list_customer_config()` DB method

**File:** `crates/mika-agent/src/db.rs`

```rust
pub fn list_customer_config(&self) -> Result<Vec<(String, String)>> {
    let mut stmt = self.conn.prepare("SELECT key, value FROM customer_config ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}
```

**File:** `crates/mika-agent/src/async_db.rs`

```rust
pub async fn list_customer_config(&self) -> Result<Vec<(String, String)>> {
    self.with_db(|db| db.list_customer_config()).await
}
```

#### 2.2 Pass `args` to `handle_config`

**File:** `crates/mika-cli/src/tui/commands/handlers.rs`

- [ ] Change dispatch: `"config" | "cfg" => Some(handle_config(app, args).await)`
- [ ] Change signature: `async fn handle_config(app: &mut App<'_>, args: &str) -> String`

#### 2.3 Implement `/config set <key> <value>`

**File:** `crates/mika-cli/src/tui/commands/handlers.rs`

```rust
const SETTABLE_CONFIG_KEYS: &[&str] = &["chat_id", "timezone"];

async fn handle_config(app: &mut App<'_>, args: &str) -> String {
    if args.starts_with("set") {
        return handle_config_set(app, &args[3..].trim()).await;
    }
    // Existing config display logic...
    // Plus: append customer_config entries
    let mut output = /* existing config display */;
    if let Ok(configs) = app.db.list_customer_config().await {
        if !configs.is_empty() {
            output.push_str("\n--- Customer Config ---\n");
            for (key, value) in &configs {
                output.push_str(&format!("  {key}: {value}\n"));
            }
        }
    }
    output
}

async fn handle_config_set(app: &mut App<'_>, args: &str) -> String {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return "Usage: /config set <key> <value>\nSettable keys: chat_id, timezone".to_string();
    }
    let (key, value) = (parts[0], parts[1]);
    if !SETTABLE_CONFIG_KEYS.contains(&key) {
        return format!("Unknown config key: {key}\nSettable keys: {}", SETTABLE_CONFIG_KEYS.join(", "));
    }
    // Validate timezone if setting timezone
    if key == "timezone" {
        if value.parse::<chrono_tz::Tz>().is_err() {
            return format!("Invalid timezone: {value}\nExample: Asia/Singapore, America/New_York");
        }
    }
    match app.db.set_customer_config(key, value).await {
        Ok(()) => format!("Set {key} = {value}"),
        Err(e) => format!("Failed to set {key}: {e}"),
    }
}
```

#### 2.4 Update `/config` help text

**File:** `crates/mika-cli/src/tui/commands/mod.rs`

- [ ] Add `args_hint: Some("[set <key> <value>]")` to config command entry in `COMMANDS` array

### Phase 3: Cross-Channel Real-Time Polling

#### 3.1 Add `load_messages_after()` DB method

**File:** `crates/mika-agent/src/db.rs`

```rust
/// Load messages with id > after_id, optionally filtered by channel type.
/// Returns messages in ascending id order.
pub fn load_messages_after(
    &self,
    after_id: i64,
    channel_types: Option<&[&str]>,
) -> Result<Vec<ConversationMessage>> {
    if let Some(types) = channel_types {
        let placeholders: String = (0..types.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT id, role, content, channel_type, created_at FROM conversations \
             WHERE id > ?1 AND role != 'summary' AND channel_type IN ({placeholders}) \
             ORDER BY id ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(after_id)];
        for t in types {
            params.push(Box::new(t.to_string()));
        }
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), |row| {
            Ok(ConversationMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                channel_type: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    } else {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, channel_type, created_at FROM conversations \
             WHERE id > ?1 AND role != 'summary' ORDER BY id ASC"
        )?;
        let rows = stmt.query_map([after_id], |row| {
            Ok(ConversationMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                channel_type: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }
}
```

**File:** `crates/mika-agent/src/async_db.rs`

```rust
pub async fn load_messages_after(
    &self,
    after_id: i64,
    channel_types: Option<Vec<String>>,
) -> Result<Vec<ConversationMessage>> {
    self.with_db(move |db| {
        let refs: Option<Vec<&str>> = channel_types
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        db.load_messages_after(after_id, refs.as_deref())
    })
    .await
}
```

#### 3.2 Add `max_message_id()` DB method

**File:** `crates/mika-agent/src/db.rs`

```rust
pub fn max_message_id(&self) -> Result<i64> {
    Ok(self.conn.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM conversations",
        [],
        |row| row.get(0),
    )?)
}
```

**File:** `crates/mika-agent/src/async_db.rs`

```rust
pub async fn max_message_id(&self) -> Result<i64> {
    self.with_db(|db| db.max_message_id()).await
}
```

#### 3.3 Add polling state to `App` and poll in `tick()`

**File:** `crates/mika-cli/src/tui/app.rs`

Add fields to `App`:

```rust
pub last_seen_msg_id: i64,
pub poll_interval_ticks: u64, // ~167 for 5s at 30ms tick
```

**File:** `crates/mika-cli/src/commands/chat.rs`

Initialize after history load:

```rust
// After loading history messages into app.messages
app.last_seen_msg_id = worker._ctx.async_db.max_message_id().await.unwrap_or(0);
app.poll_interval_ticks = 167; // ~5 seconds at 30ms tick rate
```

**File:** `crates/mika-cli/src/tui/app.rs` (in `tick()`)

Add polling logic:

```rust
// Cross-channel polling (every ~5 seconds, only when idle)
if self.tick_count % self.poll_interval_ticks == 0
    && self.status == AgentStatus::Idle
{
    self.poll_cross_channel_messages().await;
}
```

Implement `poll_cross_channel_messages()`:

```rust
async fn poll_cross_channel_messages(&mut self) {
    let channels = vec!["telegram".to_string()];
    if let Ok(new_msgs) = self.db.load_messages_after(
        self.last_seen_msg_id,
        Some(channels),
    ).await {
        if new_msgs.is_empty() {
            return;
        }
        for msg in &new_msgs {
            let role = match msg.role.as_str() {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => continue,
            };
            self.messages.push(ChatMessage {
                role,
                content: msg.content.clone(),
                rendered: None,
                channel: Some(msg.channel_type.clone()),
            });
        }
        // Update watermark
        if let Some(last) = new_msgs.last() {
            self.last_seen_msg_id = last.id;
        }
        // Auto-scroll only if user is at bottom
        if self.scroll_offset == 0 {
            // Already at bottom, stay there
        }
        self.needs_redraw = true;
    }
}
```

Also update watermark after processing agent response (in `tick()` after response reveal completes):

```rust
// After reveal complete block (around line 346):
// Update watermark to avoid re-polling our own messages
if let Ok(max_id) = self.db.max_message_id().await {
    self.last_seen_msg_id = max_id;
}
```

## Files Modified

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/app.rs` | Add `channel` to ChatMessage, polling fields, `poll_cross_channel_messages()` |
| `crates/mika-cli/src/commands/chat.rs` | History filter `["cli","telegram"]`, set channel, init watermark |
| `crates/mika-cli/src/tui/ui.rs` | Channel prefix rendering `[telegram]` in yellow via `channel_prefix_span()` |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `/config set` with allowlist, show customer_config, timezone validation |
| `crates/mika-cli/src/tui/commands/mod.rs` | Update config `args_hint` |
| `crates/mika-agent/src/db.rs` | Add `load_messages_after()`, `max_message_id()`, `list_customer_config()` |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for 3 new DB methods |

## Edge Cases Addressed

- **Polled messages during agent processing**: Only poll when `AgentStatus::Idle` to avoid mid-response confusion
- **Scroll position preservation**: Don't auto-scroll when `scroll_offset > 0` (user is reading history)
- **Timezone validation**: Validate against `chrono_tz::Tz::from_str()` before storing
- **Config allowlist**: Only `chat_id` and `timezone` settable, preventing config injection
- **Agent switch**: Watermark re-init needed (handled by `chat.rs` reload flow)
- **Export**: `/export` will naturally include channel info since `app.messages` now has it
- **Empty DB**: `max_message_id()` returns 0 via `COALESCE`

## Acceptance Criteria

- [x] `cargo test` -- all existing tests pass
- [x] `cargo clippy` -- no new warnings
- [x] New tests for `load_messages_after()` (with and without channel filter)
- [x] New tests for `max_message_id()` (empty DB, populated DB)
- [x] New tests for `list_customer_config()` (empty, populated)
- [x] New tests for `handle_config_set()` (valid key, invalid key, timezone validation, empty args)
- [ ] Manual: run TUI, `/config set chat_id 12345`, verify `/config` shows it
- [ ] Manual: run TUI + server simultaneously, send Telegram message, verify it appears in TUI within 5s with `[telegram]` prefix
- [ ] Manual: verify scroll position preserved when polling while scrolled up

## References

- Existing channel awareness pattern: `crates/mika-agent/src/prompt.rs:113-135` (VALID_CHANNELS, write_channel_section)
- Async DB wrapper pattern: `crates/mika-agent/src/async_db.rs` (closure-based dispatch)
- TUI scroll fix: `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md`
- Telegram gateway design: `docs/solutions/integration-issues/telegram-webhook-gateway-design.md`
- Prior TUI bug fixes: `docs/solutions/runtime-errors/tui-log-corruption-and-empty-agent-replies.md`
