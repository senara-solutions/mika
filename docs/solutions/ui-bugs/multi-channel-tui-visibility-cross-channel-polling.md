---
title: "Multi-channel TUI visibility, cross-channel polling, and config set"
date: 2026-02-26
module: mika-cli, mika-agent
severity: medium
tags: [tui, multi-channel, polling, telegram, config, watermark]
---

# Multi-Channel TUI Visibility, Cross-Channel Polling, and Config Set

## Problem

Three related issues prevented the TUI from being a complete multi-channel interface:

1. **TUI only loaded CLI messages on startup** — The history filter was `Some(vec!["cli".to_string()])`, making Telegram messages invisible in the TUI despite the agent seeing them.

2. **Messages received via mika-server while TUI was open didn't appear** — No polling mechanism existed to detect new cross-channel messages in real-time.

3. **No way to configure Telegram chat_id from the CLI** — Only settable via the server handler, preventing CLI-only setup workflows.

## Root Cause

The `chat.rs` history loader hardcoded `["cli"]` as the channel filter. The `ChatMessage` struct had no `channel` field, and the rendering code displayed all messages identically with no channel distinction. There was no polling mechanism and no `/config set` command.

## Solution

### Phase 1: Multi-Channel History

- Added `channel: Option<String>` to `ChatMessage` struct — `None` = CLI (default), `Some("telegram")` = Telegram
- Changed history filter to `["cli", "telegram"]`
- Maps `channel_type` from DB: CLI becomes `None`, others become `Some(channel_type)`

### Phase 2: Channel Prefix Rendering

- Extracted `channel_prefix_span()` helper in `ui.rs`
- Yellow `[telegram]` prefix on both User and Assistant messages from non-CLI channels
- System/Command/Thinking messages don't show prefix (always local)

### Phase 3: Cross-Channel Polling

- Watermark-based polling via `last_seen_msg_id` (monotonic SQLite AUTOINCREMENT ids)
- Polls every ~5 seconds (167 ticks at 30ms tick rate) using `POLL_INTERVAL_TICKS` constant
- Only polls when `AgentStatus::Idle` to avoid mid-response visual confusion
- Updates watermark after agent response reveal completes to avoid re-polling own messages
- Preserves scroll position (no auto-scroll when `scroll_offset > 0`)
- Logs poll errors via `tracing::warn!` for operational visibility

### Phase 4: /config set Command

- `SETTABLE_CONFIG_KEYS` allowlist: `["chat_id", "timezone"]`
- `chat_id` validated as `i64` (Telegram chat IDs are integers)
- `timezone` validated via `chrono_tz::Tz::from_str()`
- 1000-char length limit on all config values
- Uses `strip_prefix("set")` for safe string handling
- `/config` display shows customer settings from `list_customer_config()`

## Key Patterns

### Watermark Polling Pattern

```rust
const POLL_INTERVAL_TICKS: u64 = 167; // ~5 seconds at 30ms tick rate

// In tick():
if self.tick_count % POLL_INTERVAL_TICKS == 0 && self.status == AgentStatus::Idle {
    self.poll_cross_channel_messages().await;
}

// In poll method:
let new_msgs = self.db.load_messages_after(self.last_seen_msg_id, Some(channels)).await;
if let Some(last) = new_msgs.last() {
    self.last_seen_msg_id = last.id;
}
```

Initialize watermark from `max_message_id()` after history load to avoid re-fetching history on first poll.

### Option<String> for Optional Channel

Using `None` for CLI (the common case) and `Some("telegram")` for others:
- Minimizes constructor changes (just add `channel: None`)
- Zero visual noise for local messages
- Extensible for future channels (WhatsApp)

### Config Allowlist Pattern

```rust
const SETTABLE_CONFIG_KEYS: &[&str] = &["chat_id", "timezone"];

if !SETTABLE_CONFIG_KEYS.contains(&key) {
    return format!("Unknown config key: {key}\nSettable keys: {}", ...);
}
// Per-key validation (chat_id as i64, timezone via chrono_tz)
```

## Prevention

- When adding display-only features, check if the underlying data source covers all relevant channels
- When building real-time features on shared SQLite, use watermark-based polling (WAL mode enables concurrent read/write)
- Validate config values at the boundary with domain-specific checks, not just allowlist membership
- Extract rendering helpers early when the same visual pattern appears for multiple roles

## Files Modified

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/app.rs` | ChatMessage channel field, polling state, poll method |
| `crates/mika-cli/src/commands/chat.rs` | Multi-channel history filter, watermark init |
| `crates/mika-cli/src/tui/ui.rs` | `channel_prefix_span()` helper, channel prefix rendering |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `/config set` with allowlist, validation, display |
| `crates/mika-cli/src/tui/commands/mod.rs` | Config args_hint |
| `crates/mika-cli/src/tui/input.rs` | channel: None for clipboard messages |
| `crates/mika-agent/src/db.rs` | `load_messages_after()`, `max_message_id()`, `list_customer_config()` |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for 3 new methods |

## Related

- Prior TUI scroll fix: `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md`
- Telegram gateway design: `docs/solutions/integration-issues/telegram-webhook-gateway-design.md`
- TUI log corruption: `docs/solutions/runtime-errors/tui-log-corruption-and-empty-agent-replies.md`
