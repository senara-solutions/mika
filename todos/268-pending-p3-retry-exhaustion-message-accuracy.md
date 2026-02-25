---
status: pending
priority: p3
issue_id: 268
tags: [code-review, quality, api-client]
dependencies: []
---

# Retry-exhaustion path always says "busy" regardless of error type

## Problem Statement

The retry-exhaustion path in `claude.rs` always wraps `last_error` with "Claude API is busy. Please wait a moment and try again." However, `is_retryable()` retries on 429 (rate limit), 500 (server error), and 529 (overloaded). If retries exhaust on a 500, saying "busy" is misleading — "temporarily unavailable" would be more accurate.

## Findings

- **File**: `crates/mika-common/src/claude.rs:234-239`
- **Impact**: Low — the user action is the same ("try again later") regardless of the exact error
- **Found by**: code-simplicity-reviewer, pattern-recognition-specialist, agent-native-reviewer

## Proposed Solutions

### Option A: Match last error type (Recommended)
```rust
Err(last_error
    .map(|e| match &e {
        ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
            anyhow::Error::from(e).context(
                "Claude API is temporarily unavailable. Please try again shortly.",
            )
        }
        _ => {
            anyhow::Error::from(e).context(
                "Claude API is busy. Please wait a moment and try again.",
            )
        }
    })
    .unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
```

- Pros: More accurate messaging
- Cons: Slightly more complex
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] 429 retry exhaustion says "busy"
- [ ] 500/529 retry exhaustion says "temporarily unavailable"

## Work Log

| Date | Action | Notes |
|------|--------|-------|
| 2026-02-25 | Created | Found during PR #15 review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/15
