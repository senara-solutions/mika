---
status: pending
priority: p2
issue_id: "012"
tags: [code-review, security, telegram]
dependencies: []
---

# Telegram Webhook Missing Secret Token Verification

## Problem Statement

The Telegram webhook endpoint does not use `secret_token` parameter in `set_webhook()` or verify the `X-Telegram-Bot-Api-Secret-Token` header. Anyone who discovers the webhook URL can inject fake updates.

## Findings

- **Source:** Security Sentinel (H4)
- **Location:** `app/api/main.py` — `telegram_webhook()` and `bot.set_webhook()`

## Proposed Solutions

### Option A: Add secret_token to webhook setup and verify header (Recommended)
- Generate/configure secret token; pass to `set_webhook(secret_token=...)`
- Verify `X-Telegram-Bot-Api-Secret-Token` header in webhook handler
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Telegram webhook uses `secret_token` parameter
- [ ] Webhook handler verifies the secret token header
- [ ] Invalid/missing tokens return 403

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
