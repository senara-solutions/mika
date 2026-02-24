---
status: ready
priority: p3
issue_id: "165"
tags: [code-review, data-integrity, ux]
---

# Improve Error Message for Duplicate chat_id Pairing

## Problem Statement
If a user rapidly clicks two different pairing links, the UNIQUE constraint on `telegram_chat_id` causes the second UPDATE to fail. The user sees "I'm having trouble right now" instead of "This Telegram account is already linked to another account." (routes.rs:269)

## Findings
- **Data integrity guardian**: Catch Postgres error code 23505 (unique violation) specifically

## Proposed Solutions

### Option A: Catch unique violation error (Recommended)
```rust
Err(e) => {
    if let Some(db_err) = e.as_database_error() {
        if db_err.code().as_deref() == Some("23505") {
            let _ = state.telegram.send_message(chat_id, "This Telegram account is already linked.").await;
            return;
        }
    }
    // ... existing transient error handling
}
```
- Effort: Small (15 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (handle_pairing)

## Acceptance Criteria
- [ ] Unique violation on telegram_chat_id produces appropriate user message
- [ ] Other DB errors still show transient error

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
