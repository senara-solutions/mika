# Delegated Agent Telegram Delivery + Agent Identification + Reply Routing

**Date:** 2026-03-14
**Status:** Decided
**Issue:** [#149](https://github.com/senara-solutions/mika/issues/149)

## What We're Building

Three related improvements to multi-agent Telegram messaging:

1. **Fix delegated agent message delivery** — When mika-dev delegates a task to mika that requires sending a Telegram message, the `send_message` tool silently fails because `message_sender: None` is hardcoded in the team agent context.
2. **Agent identification** — When multiple agents send messages to the user via Telegram, there's no way to tell which agent sent what. Outbound messages are now prefixed with `[agent_name]`.
3. **Reply routing** — When the user replies to a specific agent's Telegram message, the gateway now routes that reply to the correct agent container by looking up the original message's agent in the `outbound_messages` table.

### Observed Behavior

1. User asks mika-dev (via Telegram) to send a text → mika-dev delegates to mika → mika's `send_message` returns success with warning "No outbound sender configured" → message never delivered
2. Messages from heartbeat/scheduled tasks that fail to send get queued in `failed_sends` and only flush when a new inbound Telegram message arrives (opportunistic flush)
3. TUI doesn't display messages from background tasks (heartbeat) in real-time — they only appear after TUI restart

## Why This Approach

Wire `message_sender` through `TeamAgentParams`, mirroring the existing `SilentAgentParams` pattern. This is the simplest fix that directly addresses the root cause identified in #149.

## Key Decisions

- **Wire message_sender through TeamAgentParams** — Add `message_sender: Option<Arc<dyn MessageSender>>` to `TeamAgentParams`, pass the caller's sender from `DelegateTaskTool`, query `chat_id` from DB for `telegram_configured`. Follows `SilentAgentParams` precedent.
- **Investigate TUI cross-session polling** — The TUI polls every ~5s via `poll_cross_channel_messages()` using `load_messages_after(last_seen_msg_id)`, but messages from system sessions (heartbeat) don't appear until restart. May be a session scoping issue in the query. Investigate during implementation; fix only if confirmed as a bug.
- **Defer failed_sends background flush** — Currently flush only triggers on inbound messages. A background flush task would help, but the problem hasn't been frequent enough to prioritize. Revisit if it recurs.

## Scope

### In Scope

- Add `message_sender` to `TeamAgentParams`
- Wire it through `run_team_agent_inner_impl` and `DelegateTaskTool`
- Set `telegram_configured` based on actual `chat_id` presence in DB
- Add `agent_name` to `GatewayMessageSender` and gateway `SendPayload`
- Prepend `[agent_name]` to outbound Telegram messages
- Return `message_id` from Telegram `sendMessage` API
- Store outbound message→agent mapping in `outbound_messages` Postgres table
- Parse `reply_to_message` from Telegram updates
- Route replies to the correct agent via `outbound_messages` lookup
- Periodic cleanup of old outbound message mappings (7-day retention)
- Investigate TUI cross-session message polling

### Design Note: Dual Output Paths

With `telegram_configured: true`, delegated agents now have two output paths: (1) return text to the calling agent, and (2) `send_message` directly to the user via Telegram. This is the desired behavior — the whole point of this fix — but worth noting that the system prompt will include "Telegram integration is active" guidance. The delegated agent's team context ("You are being consulted by another agent") provides sufficient framing to prefer returning results to the caller, while still allowing explicit user messaging when the task requires it.

### Out of Scope

- Background failed_sends flush (deferred)
- Push-based TUI notifications (polling at 5s is sufficient)
- Team engine call sites passing message_sender (separate concern per #149)
