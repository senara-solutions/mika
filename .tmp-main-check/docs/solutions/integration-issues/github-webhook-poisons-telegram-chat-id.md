---
title: "GitHub webhook forwarding poisons stored Telegram chat_id with zero"
date: 2026-04-17
category: integration-issues
module: crates/mika-gateway/src/github.rs
problem_type: integration_issue
component: tooling
symptoms:
  - "Telegram send failed chat_id=0 error='Bad Request: chat not found'"
  - "502 Bad Gateway on gateway /send endpoint after GitHub webhook"
  - "Owner user receives no Telegram notifications after GitHub event"
root_cause: missing_validation
resolution_type: code_fix
severity: high
tags:
  - telegram
  - chat-id
  - github-webhook
  - gateway
  - message-delivery
  - data-poisoning
---

# GitHub webhook forwarding poisons stored Telegram chat_id with zero

## Problem

After receiving a GitHub webhook, all outbound Telegram notifications fail with `chat_id=0`. The gateway's GitHub webhook forwarding hardcoded `"chat_id": 0` in the payload sent to agent containers, and the agent's `handle_message` unconditionally stored this value, overwriting the real Telegram chat_id.

## Symptoms

- Gateway logs: `WARN telegram send failed chat_id=0 error="Bad Request: chat not found"`
- Gateway returns `502 Bad Gateway` on `POST /send`
- Owner user stops receiving Telegram messages after any GitHub webhook event
- Problem self-heals temporarily when the next Telegram message arrives (restores the real chat_id)

## What Didn't Work

- The issue appeared intermittent because any inbound Telegram message would restore the correct chat_id, masking the root cause until GitHub webhook volume was high enough to consistently re-poison it.

## Solution

Three-part fix across the gateway and agent crates:

**1. Make `MessageRequest.chat_id` optional** (`crates/mika-agent/src/server/types.rs`):

Changed `chat_id: i64` to `chat_id: Option<i64>` with `#[serde(default)]`. Non-Telegram channels (GitHub webhooks) omit the field entirely; it deserializes as `None`.

**2. Guard storage in `handle_message`** (`crates/mika-agent/src/server/handlers.rs`):

Only store `chat_id` in `customer_config` when present AND non-zero:

```rust
if let Some(chat_id) = req.chat_id
    && chat_id != 0
{
    let _ = agent_state
        .db
        .set_customer_config("chat_id", &chat_id.to_string())
        .await;
}
```

**3. Remove hardcoded zero from GitHub payload** (`crates/mika-gateway/src/github.rs`):

Removed `"chat_id": 0` from the `serde_json::json!` payload in `forward_to_resolved_route`. GitHub events have no Telegram chat context — the field should not be present at all.

**4. Add validation at gateway /send** (`crates/mika-gateway/src/routes.rs`):

Added `chat_id == 0` validation in `handle_send`, returning 400 before calling the Telegram API. Defense-in-depth against any future source of zero chat_ids.

## Why This Works

Telegram chat IDs are always non-zero: positive for private chats, negative for groups/channels. The gateway was using `0` as a sentinel value for "no Telegram context," but the agent treated it as a real chat_id. Making the field `Option<i64>` makes the "absent" state explicit at the type level, and the guard prevents any non-Telegram channel from corrupting the stored value.

## Prevention

- **Type-level optionality over sentinel values**: Use `Option<T>` for fields that may be absent, not magic zero/negative sentinels. Sentinels leak through to storage and downstream consumers.
- **Guard storage at the write boundary**: Validate before storing to `customer_config`, not just before reading. The agent's unconditional `set_customer_config` was the amplifier — a single webhook poisoned all subsequent outbound delivery.
- **Defense-in-depth at trust boundaries**: The gateway `/send` endpoint now validates `chat_id != 0` even though the primary fix prevents zero from being stored. Multiple layers catch the same class of bug.
- **Deploy order matters for wire format changes**: When changing a field from required to optional between two services, deploy the consumer (agent) first so it can accept both old and new payloads.

## Related Issues

- #580 — GitHub issue tracking this bug
- `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md` — Related: agent-scoped `customer_config` chat_id lookup patterns
