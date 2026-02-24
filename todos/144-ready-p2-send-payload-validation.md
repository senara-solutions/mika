---
status: ready
priority: p2
issue_id: "144"
tags: [plan-review, security]
dependencies: []
---

# /send endpoint payload validation — text size limit and chat_id scoping

## Problem Statement
The plan's /send endpoint accepts text from agent containers but does not specify maximum text size or chat_id authorization. A compromised container could send arbitrarily large payloads or messages to other customers' chat_ids.

**Why it matters:** Without validation, a single compromised container can abuse the gateway to spam any Telegram user or exhaust gateway resources with oversized payloads.

## Findings
- Source: Security Sentinel (H-5, M-1), Performance Oracle
- The existing container server validates text at 50,000 chars — gateway /send has no equivalent
- No per-container chat_id authorization — container could specify any chat_id
- Combines with missing body size limits (todo #138) for resource exhaustion

## Proposed Solutions

### Option 1: Text size limit + chat_id scoping (Recommended)
- Validate text field: max 50,000 chars (matching container limit)
- Look up the requesting container's customer record and verify the chat_id matches
- Use the internal_token + source IP or customer_id header to identify which container is calling
- **Pros**: Prevents abuse, follows existing validation patterns
- **Cons**: Requires container identity in /send request
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan Phase 3.2 (routes.rs), /send handler

## Acceptance Criteria
- [ ] /send rejects text > 50,000 chars with 400
- [ ] /send validates chat_id belongs to requesting container's customer
- [ ] Error responses don't leak internal details

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Security Sentinel flagged unbounded /send payloads and missing chat_id authorization
