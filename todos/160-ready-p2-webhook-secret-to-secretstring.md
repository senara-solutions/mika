---
status: ready
priority: p2
issue_id: "160"
tags: [code-review, security]
---

# Change webhook_secret From String to SecretString

## Problem Statement
The `telegram_webhook_secret` is stored as plain `String` in both `GatewaySettings` (settings.rs:15) and `AppState` (routes.rs:34), while `bot_token` and `internal_token` use `SecretString`. Inconsistent — half the secrets are zeroized on drop, half aren't.

## Findings
- **Security sentinel**: Defense-in-depth gap; plain String in memory dumps, refactor risk if Debug derives change

## Proposed Solutions

### Option A: Change to SecretString (Recommended)
```rust
// settings.rs
pub telegram_webhook_secret: SecretString,
// routes.rs
pub webhook_secret: SecretString,
// In handle_webhook:
if !constant_time_eq(secret, state.webhook_secret.expose_secret()) {
```
- Effort: Small (15 min)
- Risk: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/settings.rs`, `crates/mika-gateway/src/routes.rs`

## Acceptance Criteria
- [ ] All secrets use SecretString consistently
- [ ] Debug impls remain unchanged (already redact)
- [ ] All existing tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
