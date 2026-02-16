---
status: pending
priority: p2
issue_id: "011"
tags: [code-review, security]
dependencies: []
---

# No Rate Limiting on Auth Endpoints

## Problem Statement

Login and registration endpoints have no rate limiting, allowing brute-force password attacks.

## Findings

- **Source:** Security Sentinel (H3)
- **Location:** `app/api/routes/auth.py`

## Proposed Solutions

### Option A: Add rate limiting middleware/decorator (Recommended)
- Use `slowapi` or custom Redis-based rate limiter on `/auth/login` and `/auth/register`
- Limit to ~5 attempts per minute per IP
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Login endpoint rate-limited (e.g., 5/min per IP)
- [ ] Registration endpoint rate-limited
- [ ] Rate limit responses return 429 with Retry-After header

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
