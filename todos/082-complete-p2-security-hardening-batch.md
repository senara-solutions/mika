---
status: pending
priority: p2
issue_id: "082"
tags: [code-review, security]
dependencies: []
---

# Security hardening: PII logging, input validation, transactions

## Problem Statement
Multiple security improvements identified across the codebase: message content logged at INFO level (PII exposure), `fire_at` field not length-validated, mutex poisoning crashes entire container, manual transaction management error-prone.

## Findings
1. **PII in logs** — send_message.rs:59 and messaging.rs:17 log full message text at INFO level. In production with centralized logging, user PII (schedules, financial info) written to log aggregation.
2. **fire_at validation** — create_reminder.rs:43-67 validates emptiness and ISO 8601 format but not length. Convention requires MAX_INPUT_LEN check on all string inputs.
3. **Mutex poisoning** — async_db.rs uses `.lock().expect()` in 40+ places. Any panic while holding lock permanently poisons mutex, crashing all subsequent operations.
4. **Manual transactions** — db.rs:913-927 replace_with_summary uses raw BEGIN/COMMIT. If COMMIT fails, error is silently discarded.
5. **timezone_offset** — db.rs:776-785 `count_heartbeat_sends_today` accepts free-form string as SQLite modifier. Invalid modifier returns NULL silently, bypassing rate limiting.

## Proposed Solutions
### Option 1: Fix all items
1. Log message content at `debug` level, log only `text_len` at info
2. Add `fire_at.len() > 64` guard before parsing
3. Replace `.expect()` with `.unwrap_or_else(|e| e.into_inner())` or use `parking_lot::Mutex`
4. Add comment explaining `&self` constraint prevents `conn.transaction()`; consider `&mut self` for Phase 2
5. Validate timezone_offset format with regex before use

**Effort:** 1 hour total | **Risk:** Low

## Acceptance Criteria
- [ ] Message content not logged at INFO level
- [ ] fire_at validated for max length
- [ ] Mutex poisoning handled gracefully
- [ ] Transaction limitation documented
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
