---
title: "Multi-agent Telegram messaging — delegated delivery, agent identification, and reply routing"
date: 2026-03-14
category: integration-issues
severity: high
tags:
  - multi-agent
  - delegation
  - telegram
  - message-routing
  - agent-identification
modules:
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/messaging.rs
  - crates/mika-agent/src/tools/delegate_task.rs
  - crates/mika-agent/src/teams/engine.rs
  - crates/mika-gateway/src/routes.rs
  - crates/mika-gateway/src/telegram.rs
issue: "#149"
problem_type: integration-issues
---

# Multi-Agent Telegram Messaging: Delegated Delivery, Agent Identification, and Reply Routing

## Problem

Three related issues prevented multi-agent Telegram messaging from working:

1. **Delegated agents can't send messages (#149):** When an orchestrator (e.g., `mika-dev`) delegates a task to another agent (e.g., `mika`) via `delegate_task`, the delegate's `send_message` tool silently fails because `TeamAgentParams` hardcoded `message_sender: None` and `telegram_configured: false`.

2. **No agent identification:** When multiple agents send messages to the user, there's no way to distinguish who sent what.

3. **No reply routing:** When a user replies to a specific agent's Telegram message, the system can't route that reply to the correct agent.

## Root Cause

`TeamAgentParams` (used by `delegate_task` and the team engine) hardcoded `message_sender: None`, preventing delegated agents from sending outbound messages. The `telegram_configured` flag in the system prompt was also hardcoded to `false`, so the agent wouldn't even attempt to use `send_message`. No infrastructure existed in the gateway for tracking which agent sent which message.

## Solution

### Part 1: Wire message_sender through TeamAgentParams

Added `message_sender: Option<Arc<dyn MessageSender>>` to `TeamAgentParams`. In `delegate_task.rs`, the tool creates a **new** `GatewayMessageSender` with the delegate's `agent_name` — not cloning the orchestrator's sender:

```rust
let delegate_sender: Option<Arc<dyn MessageSender>> =
    if ctx.message_sender.is_some() {
        if let (Some(url), Some(token)) = (&self.settings.routing_url, &self.settings.internal_token) {
            Some(Arc::new(GatewayMessageSender::new(
                url.clone(), token.clone(), async_db.clone(),
                reqwest::Client::new(), None,
                Some(agent_name.to_string()),  // Delegate's name, not orchestrator's
            )))
        } else { None }
    } else { None };
```

The `telegram_configured` flag now checks both `chat_id` and `message_sender`:

```rust
telegram_configured: chat_id.is_some() && params.message_sender.is_some(),
```

Team engine agents intentionally get `message_sender: None` — they communicate through the orchestrator pipeline (workspace entries, deliverables, critic feedback), not directly to users.

### Part 2: Agent Identification

`GatewayMessageSender` carries `agent_name: Option<String>`, included in the JSON payload to the gateway's `/send` endpoint. The gateway prepends `[agent_name]` to outbound Telegram messages and validates the name at the trust boundary (alphanumeric + `-` + `_`, max 64 chars).

### Part 3: Reply Routing

1. **Return message_id from Telegram:** `TelegramClient::send_message` changed from `Result<(), ...>` to `Result<i64, ...>`, returning the Telegram message_id.

2. **Store outbound mapping:** New `outbound_messages` Postgres table (`002_outbound_messages.sql`) stores `(telegram_message_id, chat_id, agent_name)` after each successful send (best-effort).

3. **Parse reply context:** Added `reply_to_message: Option<ReplyToMessage>` to `TelegramMessage` and `reply_to_message_id: Option<i64>` to `ParsedMessage` variants.

4. **Route replies:** `resolve_reply_agent()` looks up the originating agent from `outbound_messages` and includes `"agent": "<name>"` in the forwarded payload to the container.

5. **Periodic cleanup:** Every ~100 webhooks, purges records older than 7 days (batched DELETE with LIMIT 1000).

### Part 4: Explicit chat_id for Delegate Senders (Follow-up Fix)

The initial implementation had a subtle bug: the `customer_config` table uses `(agent_id, key)` as its primary key. When the handler stores `chat_id`, it stores it under the orchestrator's `agent_id` (e.g., `"mika"`). But `delegate_task` creates a new `AsyncDatabase` with the delegate's `agent_id` (e.g., `"mika-dev"`), so `GatewayMessageSender.send()` failed with "chat_id not configured" and `telegram_configured` was `false`.

Fix: added `chat_id: Option<i64>` override to `GatewayMessageSender` and `telegram_chat_id: Option<i64>` to `TeamAgentParams`. The orchestrator looks up chat_id from its own DB context and passes it explicitly:

```rust
// delegate_task.rs — look up from orchestrator's context
let chat_id: Option<i64> = ctx.db
    .get_customer_config("chat_id").await
    .ok().flatten()
    .and_then(|s| s.parse().ok());

// GatewayMessageSender.resolve_chat_id() — use override or fall back to DB
async fn resolve_chat_id(&self) -> Result<i64> {
    match self.chat_id {
        Some(id) => Ok(id),
        None => /* DB lookup */,
    }
}
```

## Key Design Decisions

1. **Fresh sender per delegation, not clone:** Cloning the orchestrator's sender would propagate the orchestrator's `agent_name`, breaking attribution and reply routing. Creating a new sender with the delegate's name is essential.

2. **Team agents get no sender:** This is intentional — team agents communicate through the orchestrator pipeline, not directly to users. Comments at both call sites document this.

3. **Best-effort outbound tracking:** INSERT failures don't break message delivery. A DB hiccup loses reply routing for one message, not the message itself.

4. **Validation at trust boundary:** `agent_name` is validated at the gateway `/send` endpoint (alphanumeric + `-` + `_`, max 64 chars) for defense-in-depth.

5. **Explicit chat_id override:** Agent-scoped `customer_config` means delegates can't look up chat_id from their own DB context. Passing it explicitly avoids cross-agent DB coupling.

## Prevention

- New `TeamAgentParams` fields that carry `Option<Arc<dyn T>>` should consider whether cloning propagates the wrong identity. Document the intent. Create a fresh instance per delegate when the trait implementation carries agent-specific state (like `agent_name`).
- **Treat `None` as deliberate:** Every `None` assignment on an `Option` field that controls user-visible behavior (message delivery, attribution) should have an inline `// Intentional: <reason>` comment. The original bug was silent because `message_sender: None` compiled without explanation.
- **Enumerate all construction sites before merging.** When adding fields to `TeamAgentParams`, search for `TeamAgentParams {` and visit every match. Current sites: `delegate_task.rs`, `teams/engine.rs` (two sites), and test utils.
- Gateway fields that are rendered to users should be validated at the `/send` boundary.
- Postgres tables that grow with message volume need cleanup strategies.
- **Agent-scoped DB lookups in delegated contexts:** When creating `AsyncDatabase` for a delegate agent, any shared config (like `chat_id`) must be passed explicitly — the delegate's agent-scoped queries won't find config stored under the orchestrator's agent_id. This is the same class of bug as [team-task-child-wrong-agent-id](../database-issues/team-task-child-wrong-agent-id.md).

## Related

- [Brainstorm: Delegated Agent Telegram Delivery](../../brainstorms/2026-03-14-delegated-agent-telegram-delivery-brainstorm.md)
- [Brainstorm: Telegram Prefix Attribution](../../brainstorms/2026-03-15-telegram-prefix-attribution-brainstorm.md)
- [Plan: Delegate Agent Telegram Chat ID Fix](../../plans/2026-03-15-001-fix-delegate-agent-telegram-chat-id-plan.md)
- [Plan: Delegate Send Message Tracing](../../plans/2026-03-15-002-fix-delegate-send-message-tracing-plan.md)
- [Agent Team Management Tools Integration](agent-team-management-tools-integration.md)
- [Team Task Child Wrong Agent ID](../database-issues/team-task-child-wrong-agent-id.md)
- [Delegation Task Guard Enforcement](../architecture-patterns/delegation-work-item-guard-enforcement.md)
- [Callback/Resume Agent Lifecycle](../architecture/callback-resume-agent-lifecycle.md)
- GitHub Issue: [#149](https://github.com/senara-solutions/mika/issues/149)
