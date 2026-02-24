---
status: ready
priority: p2
issue_id: "143"
tags: [plan-review, architecture, error-handling]
dependencies: []
---

# Add typed TelegramApiError enum following ClaudeApiError pattern

## Problem Statement
The plan does not specify a typed error enum for Telegram API interactions. The existing codebase has `ClaudeApiError` with HTTP status-code retry logic (429/500/529). The gateway should follow the same pattern for Telegram API errors (429 rate limit, 403 bot blocked, 400 bad request).

**Why it matters:** Without typed errors, Telegram API failures will be handled with string matching or ignored, leading to inconsistent retry behavior and poor observability.

## Findings
- Source: Architecture Strategist (Medium)
- Existing pattern: `ClaudeApiError` in mika-common with `is_retryable()` method
- Telegram API returns: 429 (rate limited), 403 (bot blocked by user), 400 (bad request), 401 (invalid token)
- Each status code needs different handling (retry vs log vs alert)

## Proposed Solutions

### Option 1: TelegramApiError enum matching ClaudeApiError pattern (Recommended)
```rust
#[derive(Debug, thiserror::Error)]
pub enum TelegramApiError {
    #[error("Rate limited (429), retry after {retry_after}s")]
    RateLimited { retry_after: u64 },
    #[error("Bot blocked by user (403)")]
    BotBlocked { chat_id: i64 },
    #[error("Bad request (400): {description}")]
    BadRequest { description: String },
    #[error("Unauthorized (401)")]
    Unauthorized,
    #[error("HTTP {status}: {body}")]
    Other { status: u16, body: String },
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
}
```
- **Pros**: Consistent with existing pattern, enables targeted retry logic
- **Cons**: Slightly more code upfront
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan Phase 3.5 (telegram.rs)
- **Related Components**: Outbound message delivery, error logging

## Acceptance Criteria
- [ ] TelegramApiError enum covers all common Telegram API error codes
- [ ] Retry logic respects 429 retry_after
- [ ] 403 errors logged with chat_id for investigation
- [ ] Pattern matches ClaudeApiError style

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Architecture Strategist flagged missing typed error handling for Telegram API
