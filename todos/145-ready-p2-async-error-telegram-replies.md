---
status: complete
priority: p2
issue_id: "145"
tags: [plan-review, performance]
dependencies: []
---

# Error-path Telegram replies should be async (fire-and-forget)

## Problem Statement
The plan's webhook handler sends error messages back to users (e.g., "not paired", "container unavailable") synchronously in the request path. Since Telegram has a 60-second webhook timeout, synchronous sends add latency and risk timeout if the Telegram API is slow.

**Why it matters:** Synchronous Telegram API calls in the error path can slow down webhook processing and risk Telegram retrying the update.

## Findings
- Source: Performance Oracle (Medium)
- Plan shows `send_telegram_message()` called synchronously in error paths
- Telegram's sendMessage API has variable latency (50ms-2s)
- Webhook should return 200 ASAP, handle side-effects asynchronously

## Proposed Solutions

### Option 1: Fire-and-forget via tokio::spawn (Recommended)
Wrap error-path Telegram replies in `tokio::spawn`:
```rust
tokio::spawn(async move {
    if let Err(e) = send_telegram_message(&client, chat_id, "Sorry, something went wrong").await {
        tracing::warn!("Failed to send error reply: {}", e);
    }
});
```
- **Pros**: Webhook returns immediately, no latency from Telegram API
- **Cons**: No guarantee error message is delivered (acceptable for error replies)
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan Phase 3.3 (routing.rs), webhook handler

## Acceptance Criteria
- [ ] Error-path Telegram replies do not block webhook response
- [ ] Webhook always returns 200 within milliseconds
- [ ] Failed error replies are logged but don't affect processing

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Performance Oracle flagged synchronous Telegram calls in error paths
