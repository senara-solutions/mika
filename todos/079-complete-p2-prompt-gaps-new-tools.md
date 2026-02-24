---
status: pending
priority: p2
issue_id: "079"
tags: [code-review, agent-native, prompt]
dependencies: ["074"]
---

# Add reminder tool guidance to conversation system prompt

## Problem Statement
The conversation-mode system prompt mentions core memory and structured facts but says nothing about reminders or the send_message tool. Claude sees tool definitions via the API but lacks prompt-level guidance on when to proactively offer reminders. Also, `message_sender: None` is hardcoded in conversation-mode `AgentParams`, preventing future HTTP server from threading the sender through.

## Findings
- prompt.rs:88-101 — Instructions section only mentions memory tools
- agent.rs:118 — `message_sender: None` hardcoded in conversation ToolContext
- AgentParams struct lacks `message_sender` field entirely
- send_message tool always registered but description says "prefer responding directly" in conversation mode

## Proposed Solutions
### Option 1: Add tool instructions + thread message_sender
Add to system prompt:
```
- Schedule reminders using create_reminder when user mentions deadlines.
- Use list_reminders to check active reminders.
- In silent mode, use send_message to contact user. In conversation, respond directly.
```
Add `message_sender` to `AgentParams` and thread through.
**Effort:** 30 minutes | **Risk:** Low

## Acceptance Criteria
- [ ] System prompt mentions reminder tools
- [ ] AgentParams has message_sender field
- [ ] ToolContext in conversation mode receives message_sender
- [ ] Tests updated

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
