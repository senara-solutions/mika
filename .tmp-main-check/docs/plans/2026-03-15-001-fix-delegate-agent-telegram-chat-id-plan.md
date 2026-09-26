---
title: "fix: delegated agent Telegram messages missing agent name prefix"
type: fix
status: completed
date: 2026-03-15
---

# fix: delegated agent Telegram messages missing agent name prefix

## Overview

When a delegated agent (e.g., `mika-dev`) sends a message via the `send_message` tool, the Telegram message appears without the `[mika-dev]` prefix, and replies to that message don't route back to the correct agent.

## Problem Statement

The `customer_config` table uses `(agent_id, key)` as its primary key. When the handler stores `chat_id`, it stores it under the default agent's `agent_id` (e.g., `"mika"`). When `delegate_task` creates a new `AsyncDatabase` with `agent_id="mika-dev"`, two things break:

1. **`GatewayMessageSender.send()` fails** — it calls `self.db.get_customer_config("chat_id")` which queries for `agent_id="mika-dev"`, finding no rows. Returns error: "chat_id not configured".

2. **`telegram_configured` is `false`** — `run_team_agent_inner_impl` derives this from `params.db.get_customer_config("chat_id")` which also queries against `"mika-dev"`. The system prompt omits send_message guidance, so the LLM may not even attempt to use it.

The result: the delegate's `send_message` call fails silently (error propagates via `?`), and the orchestrator may relay the message through its own sender (missing the delegate's agent name) or through its text response.

## Proposed Solution

Add an explicit `chat_id: Option<i64>` field to `GatewayMessageSender`. When set, skip the DB lookup and use it directly. This avoids cross-agent DB coupling and is the minimal change.

### Changes

#### 1. `crates/mika-agent/src/messaging.rs` — Add `chat_id` override to `GatewayMessageSender`

- Add `chat_id: Option<i64>` field to the struct
- Update `new()` to accept the parameter
- In `send()`: if `self.chat_id` is `Some(id)`, use it directly; otherwise fall back to DB lookup

#### 2. `crates/mika-agent/src/tools/delegate_task.rs` — Pass chat_id to delegate sender

- Before creating `delegate_sender`, look up `chat_id` from the orchestrator's DB: `ctx.db.get_customer_config("chat_id")`
- Parse it to `i64` and pass to `GatewayMessageSender::new()` as the `chat_id` override
- Also pass the chat_id to `TeamAgentParams` so `telegram_configured` can be set correctly

#### 3. `crates/mika-agent/src/agent.rs` — Add `telegram_chat_id` to `TeamAgentParams`

- Add `telegram_chat_id: Option<i64>` to `TeamAgentParams`
- In `run_team_agent_inner_impl`: use `params.telegram_chat_id` instead of DB lookup for `telegram_configured`

#### 4. Update all `GatewayMessageSender::new()` call sites

- `crates/mika-agent/src/server/handlers.rs` — pass `None` (handler already looks up from DB)
- `crates/mika-agent/src/server/mod.rs` — pass `None` (engine sender uses DB lookup)
- `crates/mika-agent/src/tools/delegate_task.rs` — pass `Some(chat_id)` from orchestrator's context

#### 5. Update all `TeamAgentParams` construction sites

- `delegate_task.rs` — pass `Some(chat_id)`
- `teams/engine.rs` — pass `None` (team agents intentionally have `message_sender: None`)

## Acceptance Criteria

- [x] When a delegated agent calls `send_message`, the Telegram message shows `[agent-name]` prefix
- [x] Replies to delegated agent messages route back to that agent
- [x] `telegram_configured` is `true` in the delegate's system prompt when the orchestrator has Telegram configured
- [x] Existing non-delegate paths (handler, engine sender) continue to work unchanged
- [x] All existing tests pass
- [x] New test: `GatewayMessageSender` with explicit `chat_id` skips DB lookup

## Context

- **Root cause file:** `crates/mika-agent/src/messaging.rs:80-86` — `send()` looks up chat_id per agent_id
- **Schema:** `customer_config` table has `PRIMARY KEY (agent_id, key)` — agent-scoped by design
- **Branch:** `feat/149/multi-agent-telegram-delivery`
- **Related:** `docs/solutions/integration-issues/multi-agent-telegram-delivery-and-reply-routing.md`
