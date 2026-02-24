---
status: complete
priority: p1
issue_id: "169"
tags: [code-review, security, reliability]
---

# Fix Webhook Semaphore Silent Message Loss

## Problem Statement
When the webhook semaphore is at capacity (30 permits), the handler returns `StatusCode::OK` (200) to Telegram. Telegram treats 200 as "successfully delivered" and will NOT retry. Messages are permanently and silently lost — no user feedback, no DB record, no agent notification. This is the worst outcome: data loss with no signal.

## Findings
- **Security sentinel**: MEDIUM severity — silent permanent message loss under load
- **Agent-native reviewer**: Creates invisible black hole; agent never sees shed messages
- **Performance oracle**: Returning 200 is deliberate to avoid Telegram retry storm, but creates permanent loss
- **Learnings researcher**: Existing docs recommend Semaphore with configurable capacity but don't address the return code

## Proposed Solutions

### Option A: Return 503 so Telegram retries (Recommended)
```rust
Err(_) => {
    warn!("webhook at capacity, shedding load");
    return StatusCode::SERVICE_UNAVAILABLE;
}
```
Telegram retries with exponential backoff for up to ~24 hours on non-200 responses.
- **Effort**: Trivial (2 min)
- **Risk**: Low — Telegram has built-in backoff. Could cause retry storm if sustained overload, but 503 is the correct signal.

### Option B: Return 429 with Retry-After header
```rust
Err(_) => {
    warn!("webhook at capacity, shedding load");
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", HeaderValue::from(5));
    return (StatusCode::TOO_MANY_REQUESTS, headers).into();
}
```
- **Effort**: Small (5 min)
- **Risk**: None — most explicit signal to Telegram

### Option C: Accept and queue internally
Keep returning 200 but enqueue the update into an internal bounded queue (`tokio::sync::mpsc`) for deferred processing.
- **Effort**: Medium (1-2 hours)
- **Risk**: Low — more complex but provides strongest delivery guarantee

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:102-109`
- **Telegram behavior**: Non-200 responses trigger exponential backoff retries for ~24h

## Acceptance Criteria
- [ ] Shed messages trigger Telegram retry (non-200 response)
- [ ] Log includes update_id of shed messages for monitoring
- [ ] No permanent message loss under transient load spikes

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- Telegram Bot API: webhook retry behavior on non-200
