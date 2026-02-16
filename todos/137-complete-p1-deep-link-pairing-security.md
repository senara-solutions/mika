---
status: complete
priority: p1
issue_id: "137"
tags: [plan-review, security]
dependencies: []
---

# Deep link pairing security — UUID predictability, no expiry, no rate limit

## Problem Statement
The plan's pairing flow uses the customer UUID directly as the deep link token (`/start <customer_id>`). UUIDs (even v4) are not cryptographic secrets — they can be leaked in logs, URLs, or browser history. The plan specifies no expiry on pairing tokens and no rate limiting on pairing attempts. An attacker who obtains or guesses a customer_id could pair their Telegram account to someone else's Mika container.

**Why it matters:** Pairing links a Telegram account to a customer's private AI assistant with access to all their personal data, memories, and conversations.

## Findings
- Source: Security Sentinel (C-2), Agent-Native Reviewer
- Location: Plan Phase 3.4 (pairing.rs) — `/start` deep link handling
- customer_id (UUID) used directly as pairing token
- No time-based expiry on pairing validity
- No rate limiting on failed pairing attempts
- No mechanism to revoke/regenerate pairing tokens

## Proposed Solutions

### Option 1: Dedicated pairing_token with expiry (Recommended)
Add a `pairing_token` column (random 32-byte hex, NOT the UUID) with `pairing_expires_at` timestamp. Generate token during provisioning, expire after 24h, allow regeneration.
```sql
ALTER TABLE customers ADD COLUMN pairing_token TEXT UNIQUE;
ALTER TABLE customers ADD COLUMN pairing_expires_at TIMESTAMPTZ;
```
- **Pros**: Cryptographically random token, time-limited, revocable
- **Cons**: Slightly more complex provisioning flow
- **Effort**: Small
- **Risk**: Low

### Option 2: HMAC-signed pairing links
Sign the customer_id with a server-side secret to create non-guessable links.
- **Pros**: No extra DB column needed
- **Cons**: Cannot revoke individual tokens without changing the secret
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan schema, Phase 3.4 (pairing.rs), provisioning script
- **Related Components**: Customer onboarding, Telegram bot setup

## Acceptance Criteria
- [ ] Pairing token is cryptographically random (not UUID)
- [ ] Pairing tokens expire after configurable time (default 24h)
- [ ] Failed pairing attempts are rate-limited
- [ ] Pairing tokens can be regenerated
- [ ] Tests cover expired token, invalid token, rate-limited scenarios

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Security Sentinel flagged UUID predictability and missing expiry in pairing flow
