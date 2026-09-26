---
status: complete
priority: p1
issue_id: "157"
tags: [code-review, security, input-validation]
---

# Validate Pairing Token Format Before Database Query

## Problem Statement
Pairing tokens from `/start <payload>` are trimmed but not validated for format before hitting Postgres (telegram.rs:73-81, routes.rs:225-238). Legitimate tokens are 64-char hex strings. Without validation, arbitrary-length Unicode strings hit the database on every malformed `/start` attempt.

## Findings
- **Security sentinel**: Allows SQL probing and DB load from spam `/start <garbage>`
- **Data integrity guardian**: No schema-level length constraint; recommends app-level validation

## Proposed Solutions

### Option A: Validate in handle_pairing before SQL (Recommended)
```rust
fn is_valid_pairing_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}
// In handle_pairing, before SQL:
if !is_valid_pairing_token(pairing_token) {
    let _ = state.telegram.send_message(chat_id, "Invalid or expired invite link.").await;
    return;
}
```
- Pros: Zero DB load for invalid tokens
- Cons: Couples format expectation to gateway
- Effort: Small (10 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (handle_pairing function)

## Acceptance Criteria
- [ ] Non-64-char-hex tokens rejected before database query
- [ ] Same user-facing error message ("Invalid or expired invite link.")
- [ ] Test added for format validation

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
