---
title: Fix Telegram Message Delivery and Tmux Skill Availability
type: fix
status: completed
date: 2026-02-27
---

# Fix Telegram Message Delivery and Tmux Skill Availability

## Overview

Two issues reported by the user:

1. **Telegram message not received**: User sent a message on Telegram but Mika didn't receive it. Log analysis (`~/.mika/agents/main/logs/mika.log.2026-02-27`) shows ALL activity is CLI-mode only — no server/telegram channel entries exist. The `send_message` tool logs `"CLI mode, no sender configured"`, confirming `MIKA_ROUTING_URL` / `MIKA_INTERNAL_TOKEN` env vars are not set.

2. **Tmux tools not available**: Mika explicitly says "I need to use the tmux tools directly — but I don't see them listed in my available tools." The tmux skill has `always_on = false` with keywords `["tmux", "terminal session", "background process", "long-running"]`. If the user's message doesn't contain these exact substrings, tmux tools are never injected into the agent's tool list.

## Problem Statement / Motivation

### Issue 1: Telegram Inbound/Outbound Broken in CLI Mode

**Root Cause Analysis:**

The log shows only CLI activity:
```json
{"level":"INFO","message":"send_message (CLI mode, no sender configured)"}
{"level":"INFO","message":"agent done","channel_type":"cli"}
```

Two separate problems:

- **Outbound (Mika → Telegram)**: `GatewayMessageSender` is not constructed because `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` are not set in the environment. The code in `crates/mika-cli/src/init.rs:127-161` correctly supports this — it's a configuration issue, not a code bug.

- **Inbound (Telegram → Mika)**: The CLI has no HTTP endpoint. The gateway routes messages to `POST /message` on the agent container (mika-server). If only the CLI is running, there's no HTTP server to receive inbound Telegram messages. This is an architectural gap — CLI users who want Telegram integration need mika-server running alongside.

**Evidence:** `crates/mika-agent/src/tools/send_message.rs:51-62` — when `ctx.message_sender` is `None`, the tool returns success with "Message logged locally" but nothing is actually sent.

### Issue 2: Tmux Skill Not Matched

**Root Cause:** `templates/skills/tmux/skill.toml` has `always_on = false`. The skill matching in `crates/mika-agent/src/skills/matcher.rs:3-22` does simple substring matching — only if the user's message contains one of `["tmux", "terminal session", "background process", "long-running"]` will the 6 tmux tools be injected.

The agent has no mechanism to request tools that aren't matched. Once Claude sees the tool list for a turn, it cannot ask for additional tools. This means if Mika decides mid-conversation it needs tmux (e.g., "let me run that in the background"), the tools won't be there.

**Evidence:** `crates/mika-agent/src/agent.rs:372-375` — skills matched per-turn, tools resolved from matched skills only. No fallback or retry mechanism.

## Proposed Solution

### Fix 1: Make tmux skill `always_on = true`

Tmux is a core capability — the agent should always have access to terminal management tools. The 6 tmux tools add minimal token overhead to each request but provide essential functionality.

**File:** `templates/skills/tmux/skill.toml`
**Change:** `always_on = false` → `always_on = true`

This is the simplest fix that directly resolves the reported issue. The bundled skill seeding on startup (`crates/mika-agent/src/bundled_skills.rs`) will propagate this change to all agent instances on next restart.

### Fix 2: Warn when send_message has no sender configured

Currently `send_message` returns success even when no message is actually delivered. This is misleading — the agent (and user) thinks the message was sent when it wasn't.

**File:** `crates/mika-agent/src/tools/send_message.rs`
**Change:** Return a warning message instead of success when no sender is configured:
```rust
None => {
    warn!("send_message called but no outbound sender configured");
    Ok(ToolOutput::success(
        "⚠ No outbound sender configured. Message was NOT delivered. \
         To enable Telegram delivery, set MIKA_ROUTING_URL and MIKA_INTERNAL_TOKEN.",
    ))
}
```

This gives the agent actionable information about why delivery failed, instead of silently succeeding.

## Acceptance Criteria

- [x] Tmux skill has `always_on = true` in `templates/skills/tmux/skill.toml`
- [x] `send_message` tool returns a clear warning (not fake success) when no sender is configured
- [x] `send_message` log level upgraded from `info!` to `warn!` for the no-sender path
- [x] All existing tests pass (`cargo test`)
- [x] `cargo clippy` clean (no new warnings introduced; pre-existing warnings in unrelated files)

## Technical Considerations

- **Token overhead**: Making tmux `always_on` adds 6 tool definitions (~600 tokens) to every API call. Acceptable tradeoff for core functionality.
- **Backwards compatibility**: Changing `send_message` output from success-sounding to warning could affect agent behavior — Claude may start proactively telling the user about the configuration issue. This is desirable.
- **Bundled skill propagation**: `seed_bundled_skills_if_needed()` in `crates/mika-agent/src/startup.rs` always overwrites bundled skill files, so the `always_on` change will propagate on next restart.

## References

- Log file: `~/.mika/agents/main/logs/mika.log.2026-02-27`
- Skill matching: `crates/mika-agent/src/skills/matcher.rs:3-22`
- Tmux skill config: `templates/skills/tmux/skill.toml`
- Send message tool: `crates/mika-agent/src/tools/send_message.rs:51-62`
- CLI sender init: `crates/mika-cli/src/init.rs:127-161`
- Bundled skills: `crates/mika-agent/src/bundled_skills.rs`
- Past solution: `docs/solutions/integration-issues/cli-telegram-messaging-and-skill-seeding.md`
- Past solution: `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md`
