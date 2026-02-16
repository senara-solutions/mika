---
status: pending
priority: p1
issue_id: "004"
tags: [code-review, security, google-calendar]
dependencies: []
---

# Google OAuth State Parameter Uses Raw User ID

## Problem Statement

The Google Calendar OAuth flow in `app/api/routes/calendar.py` passes the raw `user_id` as the `state` parameter. This allows an attacker to craft a callback URL with a victim's user_id, potentially linking the attacker's Google account to the victim's profile.

**Why it matters:** OAuth CSRF attack can link attacker's calendar to victim's account.

## Findings

- **Source:** Security Sentinel (C2)
- **Location:** `app/api/routes/calendar.py` — `flow.authorization_url(state=str(user_id))`
- **Evidence:** State is `str(user_id)` with no signing or session binding

## Proposed Solutions

### Option A: Sign the state parameter with itsdangerous (Recommended)
- Use `URLSafeTimedSerializer` (already in use for sessions) to sign the user_id
- Verify signature in callback; reject if invalid or expired
- **Pros:** Reuses existing dependency; prevents tampering
- **Cons:** None significant
- **Effort:** Small
- **Risk:** Low

### Option B: Store random nonce in session
- Generate random token, store in session with user_id mapping, verify on callback
- **Pros:** Standard OAuth CSRF prevention
- **Cons:** Requires server-side session state
- **Effort:** Small
- **Risk:** Low

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/api/routes/calendar.py`

## Acceptance Criteria

- [ ] OAuth state parameter is cryptographically signed
- [ ] Callback validates state signature before processing
- [ ] Tampered state values are rejected with appropriate error
- [ ] Tests cover valid and invalid state scenarios

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | itsdangerous already available |

## Resources

- OAuth 2.0 Security: State Parameter
