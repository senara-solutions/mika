---
status: pending
priority: p1
issue_id: "003"
tags: [code-review, security, whatsapp]
dependencies: []
---

# WhatsApp Webhook Missing Signature Verification

## Problem Statement

The WhatsApp webhook POST handler (`app/channels/whatsapp/handlers.py`) does not verify the `X-Hub-Signature-256` header. Anyone who discovers the webhook URL can inject fake messages, potentially triggering bot actions, data writes, and user creation.

**Why it matters:** Unauthenticated message injection into the system.

## Findings

- **Source:** Security Sentinel (C4), Data Integrity Guardian
- **Location:** `app/channels/whatsapp/handlers.py` — `whatsapp_webhook_post()`
- **Evidence:** No signature verification code exists; the `whatsapp_app_secret` setting exists but is unused

## Proposed Solutions

### Option A: Verify X-Hub-Signature-256 header (Recommended)
- Read raw request body, compute HMAC-SHA256 with app secret, compare with header
- Return 403 on mismatch
- **Pros:** Standard Meta verification; uses existing `whatsapp_app_secret` setting
- **Cons:** Minor performance overhead for HMAC computation
- **Effort:** Small
- **Risk:** Low

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/channels/whatsapp/handlers.py`

## Acceptance Criteria

- [ ] WhatsApp webhook POST verifies `X-Hub-Signature-256` header
- [ ] Requests with invalid/missing signatures return 403
- [ ] Valid requests continue to be processed normally
- [ ] Tests cover both valid and invalid signature scenarios

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | whatsapp_app_secret already in settings |

## Resources

- Meta WhatsApp Business API: Webhook Security
