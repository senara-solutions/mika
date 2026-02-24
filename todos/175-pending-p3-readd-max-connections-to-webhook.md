---
status: pending
priority: p3
issue_id: "175"
tags: [code-review, performance]
---

# Re-add max_connections to SetWebhookPayload

## Problem Statement
The `max_connections` field was removed from `SetWebhookPayload`, causing Telegram to use its default of 40 concurrent webhook deliveries. With the semaphore set to 30, Telegram can deliver up to 40 concurrent updates, but only 30 will be processed — the remaining 10 get parsed, auth-checked, and then shed. This wastes resources on the hot path.

## Findings
- **Performance oracle**: Marginal but prevents ~10 wasted JSON parses under peak load

## Proposed Solutions

### Option A: Set max_connections to match semaphore (Recommended)
```rust
struct SetWebhookPayload {
    url: String,
    secret_token: String,
    allowed_updates: Vec<String>,
    max_connections: u32,  // Set to 30-35
}
```
- **Effort**: Trivial (5 min)
- **Risk**: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/telegram.rs`

## Acceptance Criteria
- [ ] max_connections set to match or slightly exceed semaphore limit
- [ ] Telegram delivers no more concurrent updates than we can process

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- Telegram Bot API: setWebhook max_connections parameter
