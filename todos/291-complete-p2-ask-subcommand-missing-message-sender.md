---
status: complete
priority: p2
issue_id: 291
tags: [code-review, agent-native, parity]
dependencies: []
---

# `mika ask` subcommand does not wire GatewayMessageSender

## Problem Statement

The `mika ask` one-shot command at `ask.rs:43` hardcodes `message_sender: None`. If a user runs `mika ask "send hi on telegram"`, the system prompt tells the agent Telegram is available (because `chat_id` exists in DB), Claude calls `send_message`, but the tool hits the `None` branch and returns "Message delivered (CLI)" without actually delivering to Telegram. This is misleading — the agent believes it sent a Telegram message when nothing was delivered.

## Findings

- **Agent-Native Reviewer:** 12/13 capabilities are agent-accessible. The `ask` subcommand is the only gap. Creates inconsistency between `mika ask` and `mika` (TUI chat).
- **Learnings Researcher:** Past solution on "agent-skill-hallucination" documents that capability gaps between CLI and agent views cause hallucinated success.

## Proposed Solutions

### Solution A: Move `make_message_sender` to `init.rs` and call from both `chat.rs` and `ask.rs`

Extract `make_message_sender` from `chat.rs` into `init.rs` (or a shared module), then call it from both `chat.rs` and `ask.rs`.

```rust
// In init.rs:
pub fn make_message_sender(settings: &Settings, db: &AsyncDatabase) -> Option<Arc<dyn MessageSender>> { ... }

// In ask.rs:
let message_sender = crate::init::make_message_sender(&ctx.settings, &ctx.async_db);
```

- **Pros:** Consistent behavior across all CLI entry points, no duplication
- **Cons:** Adds `reqwest` usage to the `ask` path (increases one-shot latency slightly if gateway is configured)
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/ask.rs`, `crates/mika-cli/src/commands/chat.rs`, `crates/mika-cli/src/init.rs`

## Acceptance Criteria

- [ ] `mika ask "send hi on telegram"` delivers message to gateway when configured
- [ ] `mika ask` still works without gateway config (returns None, CLI-only behavior)
- [ ] No code duplication between `ask.rs` and `chat.rs`
- [ ] All existing tests pass
