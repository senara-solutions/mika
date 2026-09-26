---
status: complete
priority: p1
issue_id: 664
tags: [code-review, architecture, multi-agent, telegram]
dependencies: []
---

# Delegated Agent Messages Carry Orchestrator's agent_name

## Problem Statement

When a delegated agent sends a Telegram message via `send_message`, the message is attributed to the **orchestrator** agent (e.g., `[mika-dev]`) instead of the **delegated** agent (e.g., `[mika]`). This happens because `delegate_task.rs` clones the orchestrator's `ctx.message_sender`, which carries the orchestrator's `agent_name` in the `GatewayMessageSender`.

This breaks:
- **Agent identification**: User sees `[mika-dev]` when `mika` sent the message
- **Reply routing**: User replies route to `mika-dev` instead of `mika`

## Findings

- `crates/mika-agent/src/tools/delegate_task.rs:163` — `message_sender: ctx.message_sender.clone()` clones the orchestrator's sender with its `agent_name`
- `crates/mika-agent/src/messaging.rs:37` — `GatewayMessageSender.agent_name` is set at construction and cannot be changed after
- `crates/mika-agent/src/server/handlers.rs:218` — sender constructed with `Some(a.db.agent_id().to_string())`, which is the orchestrator's ID

Identified by: architecture-strategist, agent-native-reviewer

## Proposed Solutions

### Option A: Create a new GatewayMessageSender for the delegate
Create a new `GatewayMessageSender` in `delegate_task.rs` with the delegate's `agent_name` instead of cloning the orchestrator's sender. The delegate's `agent_name` is already available at line 55.

- **Pros**: Clean separation, correct attribution
- **Cons**: Need to pass gateway_url, internal_token, http_client through to delegate_task — may require new fields on `DelegateTaskTool` or a sender factory
- **Effort**: Medium
- **Risk**: Low

### Option B: Add a `with_agent_name()` method to GatewayMessageSender
Add a method that returns a new sender with a different `agent_name` but sharing the same connection/config.

- **Pros**: Minimal API change, easy to use at the clone site
- **Cons**: Slight complexity in messaging.rs
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option B — add `with_agent_name(&self, name: String) -> Self` to `GatewayMessageSender`, then use it in `delegate_task.rs`.

## Technical Details

- **Affected files**: `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/tools/delegate_task.rs`
- **Components**: GatewayMessageSender, DelegateTaskTool

## Acceptance Criteria

- [ ] Delegated agent messages show `[delegate-name]` prefix in Telegram
- [ ] Reply to a delegated agent's message routes to the delegate, not the orchestrator
- [ ] Orchestrator's own messages still show `[orchestrator-name]`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-14 | Identified during code review | Cloning the sender propagates the wrong agent_name |

## Resources

- PR: Delegated Agent Telegram Delivery + Agent Identification + Reply Routing
- Issue: #149
