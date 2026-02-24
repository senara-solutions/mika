---
status: ready
priority: p2
issue_id: "092"
tags: [code-review, architecture, agent-native]
dependencies: []
---

# Wire message_sender through AgentParams to conversation ToolContext

## Problem Statement
`message_sender` is hardcoded to `None` in the conversation agent loop's `ToolContext` (agent.rs:121). The `AgentParams` struct has no `message_sender` field, so there is no way to pass a sender from the HTTP server. This blocks Phase 2 where the agent needs to send outbound messages during conversation turns.

## Findings
- File: `crates/mika-agent/src/agent.rs:121` — `message_sender: None` hardcoded
- `AgentParams` struct (agent.rs:16-27) has no `message_sender` field
- Silent mode correctly receives `message_sender` via `SilentAgentParams`
- The `send_message` tool falls through to CLI fallback when sender is None
- Phase 2 HTTP handler must thread a gateway sender through AgentParams
- Flagged by: Agent-Native Reviewer (Critical for Phase 2)

## Proposed Solutions

### Option 1: Add field to AgentParams (Recommended)
```rust
pub struct AgentParams<'a> {
    // ... existing fields ...
    pub message_sender: Option<&'a dyn MessageSender>,
}
```
Then in `run_agent_inner`:
```rust
message_sender: params.message_sender,
```
CLI caller passes `None`, HTTP handler passes `Some(&gateway_sender)`.
**Pros:** One-line struct change, one-line plumbing, unblocks Phase 2
**Cons:** None
**Effort:** Trivial
**Risk:** None

## Technical Details
**Affected files:** `crates/mika-agent/src/agent.rs`, `crates/mika-agent/src/cli.rs`

## Acceptance Criteria
- [ ] `AgentParams` has `message_sender: Option<&'a dyn MessageSender>`
- [ ] `run_agent_inner` passes it to `ToolContext`
- [ ] CLI caller passes `None`
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified hardcoded None that blocks Phase 2 message delivery
