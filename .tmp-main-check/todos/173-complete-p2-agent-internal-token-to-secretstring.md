---
status: complete
priority: p2
issue_id: "173"
tags: [code-review, security]
---

# Upgrade Agent internal_token to SecretString

## Problem Statement
The gateway correctly uses `SecretString` for `internal_token` (routes.rs:34), but the agent crate stores it as plain `String` in `AppState` (server/state.rs:23), `Settings` (config.rs:40), and `GatewayMessageSender` (messaging.rs:25). Plain strings lack zeroize-on-drop protection and can leak through heap dumps, core dumps, or accidental logging.

## Findings
- **Security sentinel**: MEDIUM severity — inconsistency between gateway (SecretString) and agent (String)
- **Architecture strategist**: Now more visible after gateway was upgraded in this commit

## Proposed Solutions

### Option A: Upgrade to SecretString across agent crate (Recommended)
Update `internal_token` to `SecretString` in:
1. `crates/mika-common/src/config.rs` (Settings)
2. `crates/mika-agent/src/server/state.rs` (AppState)
3. `crates/mika-agent/src/messaging.rs` (GatewayMessageSender)
Use `expose_secret()` only at comparison and HTTP header injection points.
- **Effort**: Small (30 min)
- **Risk**: None — follow existing gateway pattern

## Technical Details
- **Affected files**: `crates/mika-common/src/config.rs`, `crates/mika-agent/src/server/state.rs`, `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/server/auth.rs`

## Acceptance Criteria
- [ ] internal_token is SecretString in all crates
- [ ] expose_secret() used only at auth comparison and HTTP header points
- [ ] Debug impls continue to redact the field

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- Reference: Gateway pattern in routes.rs:34
