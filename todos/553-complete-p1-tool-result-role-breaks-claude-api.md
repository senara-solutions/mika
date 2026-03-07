---
status: complete
priority: p1
issue_id: "553"
tags: [code-review, correctness, api]
dependencies: []
---

# tool_result Role Breaks Claude API on Subsequent Turns

## Problem Statement

Callback results are saved to the `conversations` table with `role = 'tool_result'`. When the agent loads conversation history via `load_recent_messages(20, None)`, these messages are included (the query only excludes `role = 'summary'`). They are then mapped directly into Claude API `Message` objects with `role: msg.role.clone()` — sending `role: "tool_result"` to the Claude API, which only accepts `"user"` and `"assistant"`.

This will cause a **400 API error on the next conversation turn** after any callback delivery, effectively breaking the conversation.

## Findings

- **Found by:** Performance Oracle, Architecture Strategist, Security Sentinel, Agent-Native Reviewer, Code Simplicity Reviewer (5/8 agents)
- **Location:** `crates/mika-agent/src/agent.rs:710-713` (history builder), `crates/mika-cli/src/commands/chat.rs:245` (saves with `role = "tool_result"`)
- **Evidence:** Claude Messages API spec only accepts `user` and `assistant` roles. The `load_recent_messages` filter is `AND role != 'summary'` — does not exclude `tool_result`.

## Proposed Solutions

### Option A: Transform in history-to-API serialization (Recommended)
- Keep `role='tool_result'` in the DB — it is the correct provider-agnostic internal representation
- In the history builder (`agent.rs`), transform `tool_result` rows into `user` messages with appropriate content blocks when building Claude API messages
- The DB stores what it IS (a tool result). The API adapter transforms it into what Claude expects (a user message).
- **Pros:** Provider-agnostic DB schema, future-proofs for multi-provider (OpenAI expects `role='tool'`, Anthropic expects inside `user` message), clean separation of storage vs serialization
- **Cons:** Serialization logic needed per provider
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Option A — keep `role='tool_result'` in DB, transform in the history-to-API serialization layer.

## Technical Details

- **Affected files:** `crates/mika-agent/src/agent.rs` (history builder)
- **Components:** Claude API message builder, conversation history loading

## Acceptance Criteria

- [ ] `tool_result` rows in DB are transformed to valid `user` messages when sent to Claude API
- [ ] Subsequent conversation turns after callback delivery work without API errors
- [ ] DB retains `role='tool_result'` for auditing and provider-agnostic storage
- [ ] Test: save callback result, load history, verify API messages use valid roles

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | 5/8 agents flagged this independently |
| 2026-03-07 | Approved during triage | User corrected fix direction: keep DB role, fix serialization layer. Provider-agnostic DB representation is the right design. |

## Resources

- PR: `feat/callback-tui-delivery` branch
- Claude Messages API: roles must be `user` or `assistant`
