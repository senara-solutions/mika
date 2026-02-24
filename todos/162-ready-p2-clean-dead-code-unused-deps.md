---
status: ready
priority: p2
issue_id: "162"
tags: [code-review, simplicity, quality]
---

# Remove Dead Code Fields and Unused Dependencies

## Problem Statement
Multiple `#[allow(dead_code)]` annotations and unused dependencies add noise and false coverage. Flagged by code simplicity reviewer.

## Findings
- **Code simplicity reviewer**: `request_id` field, `message_id` field, `chrono` dep, `tracing-subscriber` dep, `generate_pairing_token` function, `max_connections` in SetWebhookPayload, `AppState` Debug impl (never logged)
- **Architecture strategist**: `tracing-subscriber` is transitive via mika-common

## Items to Clean Up

1. Remove `SendPayload.request_id` field (routes.rs:337-338) — serde ignores unknown fields
2. Remove `TelegramMessage.message_id` field (telegram.rs:32-33) — never read
3. Move `generate_pairing_token` to `#[cfg(test)]` (routes.rs:386-390) — only tests use it
4. Remove `max_connections` from `SetWebhookPayload` (telegram.rs:123) — Telegram default
5. Remove `chrono` from Cargo.toml:29 — not directly imported
6. Remove `tracing-subscriber` from Cargo.toml:20 — handled by mika-common
7. Simplify retry-after header: use `HeaderValue::from(secs)` instead of string round-trip (routes.rs:320)
8. Remove redundant `http_client` from AppState; expose via `TelegramClient::http_client()` getter

## Technical Details
- **Affected files**: `Cargo.toml`, `routes.rs`, `telegram.rs`

## Acceptance Criteria
- [ ] No `#[allow(dead_code)]` annotations remain
- [ ] `chrono` and `tracing-subscriber` removed from gateway Cargo.toml
- [ ] `generate_pairing_token` moved to `#[cfg(test)]` or removed
- [ ] `cargo build` succeeds
- [ ] All tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
