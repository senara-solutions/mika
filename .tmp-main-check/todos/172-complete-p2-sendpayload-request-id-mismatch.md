---
status: complete
priority: p2
issue_id: "172"
tags: [code-review, architecture, observability]
---

# Fix SendPayload request_id Field Mismatch

## Problem Statement
The agent's `GatewayMessageSender` sends `request_id` in the JSON payload to `/send` (messaging.rs:79), but the gateway's `SendPayload` struct no longer includes `request_id` (routes.rs:390-394). Since serde defaults to ignoring unknown fields, the `request_id` is silently discarded. The entire purpose of threading `request_id` through the sender (TODO #166) is defeated — outbound messages cannot be correlated with inbound requests on the gateway side.

## Findings
- **Agent-native reviewer**: The request_id threading intent is good, but the mismatch means it's dead data on the gateway side
- **Architecture strategist**: Incomplete correlation story — gateway discards what agent sends
- **Code simplicity reviewer**: Dead data being serialized for no consumer

## Proposed Solutions

### Option A: Re-add request_id to SendPayload and log it (Recommended)
```rust
#[derive(serde::Deserialize)]
struct SendPayload {
    chat_id: i64,
    text: String,
    #[serde(default)]
    request_id: Option<String>,
}
```
Then add `request_id` to tracing spans in `handle_send`:
```rust
info!(chat_id = payload.chat_id, request_id = ?payload.request_id, "sending to telegram");
```
- **Effort**: Small (10 min)
- **Risk**: None

### Option B: Remove request_id from agent's outbound JSON
If the gateway doesn't need it, stop sending it.
- **Effort**: Small (10 min)
- **Risk**: Loses future observability potential

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:390-394`, `crates/mika-agent/src/messaging.rs:76-80`

## Acceptance Criteria
- [ ] request_id sent by agent is received and logged by gateway
- [ ] End-to-end correlation: inbound webhook → agent processing → outbound send

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- Related: TODO #166 (thread request_id through sender)
