---
title: "fix: add tracing to delegate send_message flow and improve tool description"
type: fix
status: completed
date: 2026-03-15
---

# fix: add tracing to delegate send_message flow and improve tool description

## Problem Statement

After fixing the chat_id override for delegate senders, the user reports that delegated agent messages still appear without the `[agent-name]` prefix on Telegram. The code analysis shows all paths correctly set `agent_name` and `chat_id`, but the runtime behavior contradicts this.

Two possible causes:
1. **Deployment mismatch** — the deployed binaries don't include the latest code changes
2. **LLM behavior** — the delegate agent doesn't call `send_message` and instead produces a text response, which the orchestrator relays through its own sender

## Solution

### 1. Add diagnostic tracing to delegate flow

Add `tracing::info!` at key decision points in the delegate → send_message flow so the user can verify at runtime what's happening:

- `delegate_task.rs`: Log chat_id lookup result and delegate_sender creation
- `messaging.rs`: Log the agent_name and chat_id being used in `send()` payload
- `send_message.rs`: Log whether the tool used a sender or fell back to no-sender path

### 2. Improve `send_message` tool description

The current description says "In conversation mode, prefer responding directly." — this may cause the LLM to avoid using the tool in team mode. Update to explicitly mention team/delegation mode.

## Acceptance Criteria

- [x] Tracing added to delegate_task (chat_id, sender creation)
- [x] Tracing added to GatewayMessageSender.send() (agent_name, chat_id used)
- [x] send_message tool description updated for team mode clarity
- [x] All tests pass
