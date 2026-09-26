---
status: complete
priority: p3
issue_id: 292
tags: [code-review, agent-native, ux]
dependencies: []
---

# `send_message` tool returns misleading "Message delivered (CLI)" when sender is None

## Problem Statement

When `message_sender` is `None`, the `send_message` tool at `send_message.rs:56-60` returns `ToolOutput::success("Message delivered (CLI).")`. If the system prompt tells the agent that Telegram is configured (because `chat_id` exists in DB), the agent believes it delivered a Telegram message. The word "delivered" implies the message reached the user, which is only true if they are looking at the CLI terminal.

## Findings

- **Agent-Native Reviewer:** The agent proceeds with a false belief. In silent mode (heartbeat/reminder), this means the user never receives the message.

## Proposed Solutions

### Solution A: Change the fallback message text

Replace "Message delivered (CLI)." with "Message logged locally (no outbound sender configured)." This makes the limitation explicit to the agent, who can then inform the user.

- **Pros:** Honest signal to the agent, minimal code change
- **Cons:** May cause Claude to explain the limitation to the user (which is actually desirable)
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/send_message.rs`

## Acceptance Criteria

- [ ] `send_message` with `None` sender returns a non-misleading message
- [ ] Agent correctly understands message was not delivered externally
