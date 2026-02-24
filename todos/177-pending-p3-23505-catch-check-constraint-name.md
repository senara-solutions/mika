---
status: pending
priority: p3
issue_id: "177"
tags: [code-review, quality]
---

# Narrow 23505 Unique Violation Catch to Specific Constraint

## Problem Statement
The Postgres error code 23505 catch in `handle_pairing` (routes.rs:310-317) does not distinguish which unique constraint was violated. The schema has unique constraints on both `telegram_chat_id` and `pairing_token`. A violation on `pairing_token` (token collision or admin error) would produce the wrong user message: "This Telegram account is already linked."

## Findings
- **Security sentinel**: LOW severity — unlikely in practice since pairing_token is set to NULL in the UPDATE, but could mask errors on future schema changes

## Proposed Solutions

### Option A: Check constraint name (Recommended)
```rust
if db_err.code().as_deref() == Some("23505") {
    let msg = if db_err.constraint().map_or(false, |c| c.contains("telegram_chat_id")) {
        "This Telegram account is already linked to another account."
    } else {
        "Pairing failed. Please contact support."
    };
    // send msg...
}
```
- **Effort**: Trivial (5 min)
- **Risk**: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:308-318`

## Acceptance Criteria
- [ ] Error message matches the specific constraint that was violated

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
