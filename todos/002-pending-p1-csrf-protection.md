---
status: pending
priority: p1
issue_id: "002"
tags: [code-review, security]
dependencies: []
---

# No CSRF Protection on POST Endpoints

## Problem Statement

All POST endpoints (login, export, delete, calendar disconnect) lack CSRF token validation. An attacker could craft a malicious page that submits forms to these endpoints while the user is authenticated, performing actions on their behalf.

**Why it matters:** Account deletion, data export, and authentication actions are all vulnerable to cross-site request forgery.

## Findings

- **Source:** Security Sentinel (C3)
- **Locations:**
  - `POST /auth/login` — `app/api/routes/auth.py`
  - `POST /api/privacy/export` — `app/api/routes/privacy.py`
  - `POST /api/privacy/delete` — `app/api/routes/privacy.py`
  - `POST /dashboard/calendar/disconnect` — `app/api/routes/calendar.py`

## Proposed Solutions

### Option A: CSRF tokens via `starlette-csrf` or `fastapi-csrf-protect` (Recommended)
- Add CSRF middleware that validates tokens on state-changing requests
- Include hidden CSRF token field in all HTML forms
- **Pros:** Standard approach; well-tested libraries
- **Cons:** Adds dependency; requires template updates
- **Effort:** Medium
- **Risk:** Low

### Option B: SameSite cookie + Origin header check
- Set `SameSite=Strict` on session cookies and validate `Origin` header
- **Pros:** No new dependency
- **Cons:** Less robust; older browsers may not support SameSite
- **Effort:** Small
- **Risk:** Medium

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/api/routes/auth.py`
- `app/api/routes/privacy.py`
- `app/api/routes/calendar.py`
- All dashboard templates with forms

## Acceptance Criteria

- [ ] All POST endpoints validate CSRF tokens
- [ ] HTML forms include CSRF token fields
- [ ] Cross-origin POST requests are rejected

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Security sentinel identified |

## Resources

- OWASP: Cross-Site Request Forgery Prevention
