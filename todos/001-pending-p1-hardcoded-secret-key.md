---
status: pending
priority: p1
issue_id: "001"
tags: [code-review, security]
dependencies: []
---

# Hardcoded Fallback Secret Key

## Problem Statement

`app/api/routes/auth.py` uses `settings.secret_key or "dev-secret-change-me"` as a fallback. If `SECRET_KEY` is not set in production, the application silently uses a well-known default secret, allowing any attacker to forge session cookies and impersonate any user.

**Why it matters:** Complete authentication bypass in production if env var is missing.

## Findings

- **Source:** Security Sentinel (C1), Architecture Strategist (R3), Python Code Quality (#4)
- **Location:** `app/api/routes/auth.py` — `serializer = URLSafeTimedSerializer(settings.secret_key or "dev-secret-change-me")`
- **Also affects:** `app/api/middleware.py` which uses the same pattern

## Proposed Solutions

### Option A: Fail fast on missing secret (Recommended)
- Remove fallback entirely; raise `ValueError` at startup if `SECRET_KEY` is not configured
- **Pros:** Prevents silent misconfiguration; simple
- **Cons:** App won't start without it (desired behavior)
- **Effort:** Small
- **Risk:** Low

### Option B: Generate random secret at startup with warning
- If not set, generate `secrets.token_hex(32)` and log a warning
- **Pros:** Dev-friendly
- **Cons:** Sessions invalidated on every restart; could mask prod misconfiguration
- **Effort:** Small
- **Risk:** Medium

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/api/routes/auth.py`
- `app/api/middleware.py`
- `app/config.py` (settings model)

## Acceptance Criteria

- [ ] No hardcoded secret key fallback exists in the codebase
- [ ] Application fails to start if `SECRET_KEY` is not set
- [ ] Existing tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Identified by 3 review agents |

## Resources

- OWASP: Cryptographic Failures
