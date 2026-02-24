---
status: ready
priority: p3
issue_id: "166"
tags: [code-review, architecture, observability]
---

# Thread request_id Through GatewayMessageSender

## Problem Statement
The plan explicitly noted (line 447): "Update messaging.rs to include request_id from the current session." The gateway's SendPayload accepts `request_id: Option<String>`, but the container's GatewayMessageSender never sends it. Outbound messages cannot be correlated with inbound requests.

## Findings
- **Architecture strategist**: Known gap, tracked; address before staging
- **Agent-native reviewer**: Important for production debugging

## Proposed Solutions

### Option A: Add request_id to GatewayMessageSender (Recommended)
Thread `request_id` from `MessageRequest` through to the sender payload.
- Effort: Small (20 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/server/types.rs`

## Acceptance Criteria
- [ ] GatewayMessageSender includes request_id in /send payload
- [ ] Gateway logs correlate inbound and outbound via request_id

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
