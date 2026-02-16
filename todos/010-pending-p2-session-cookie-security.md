---
status: pending
priority: p2
issue_id: "010"
tags: [code-review, security]
dependencies: []
---

# Session Cookie Missing Secure/SameSite Attributes

## Problem Statement

Session cookies set in `app/api/routes/auth.py` lack `secure=True`, `samesite="lax"`, and `httponly=True` attributes. This exposes session tokens to interception over HTTP and cross-site attacks.

## Findings

- **Source:** Security Sentinel (H1)
- **Location:** `app/api/routes/auth.py` — `response.set_cookie()` call

## Proposed Solutions

### Option A: Add all security attributes (Recommended)
- Set `secure=True` (HTTPS only), `samesite="lax"`, `httponly=True`
- Make `secure` conditional on `not settings.debug` for local dev
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Session cookie has `httponly=True`
- [ ] Session cookie has `secure=True` in production
- [ ] Session cookie has `samesite="lax"`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
