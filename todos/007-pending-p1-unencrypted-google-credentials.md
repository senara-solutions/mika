---
status: pending
priority: p1
issue_id: "007"
tags: [code-review, security, data-integrity]
dependencies: []
---

# Google Credentials Stored as Plaintext JSONB

## Problem Statement

Google OAuth credentials (access_token, refresh_token) are stored as plaintext JSONB in the `users.google_credentials` column. A database breach would expose all users' Google account tokens, allowing attackers to access their calendars and potentially other Google services.

**Why it matters:** Token theft on DB breach; compliance risk for storing third-party credentials in clear text.

## Findings

- **Source:** Security Sentinel (H5), Architecture Strategist (R2), Data Integrity Guardian
- **Location:** `app/models/user.py` — `google_credentials: Mapped[dict | None] = mapped_column(JSONB, nullable=True)`
- **Location:** `app/api/routes/calendar.py` — stores raw credential dict

## Proposed Solutions

### Option A: Encrypt at application level with Fernet (Recommended)
- Use `cryptography.fernet.Fernet` to encrypt credentials before storage
- Store as encrypted text column; decrypt on read
- Derive encryption key from a separate `ENCRYPTION_KEY` env var
- **Pros:** Standard symmetric encryption; simple implementation
- **Cons:** Need to manage encryption key; slightly more complex read/write
- **Effort:** Medium
- **Risk:** Low

### Option B: Use PostgreSQL pgcrypto extension
- Encrypt at database level using `pgp_sym_encrypt`/`pgp_sym_decrypt`
- **Pros:** Transparent to application code
- **Cons:** Requires DB extension; key management still needed
- **Effort:** Medium
- **Risk:** Low

### Option C: Use a secrets manager (Vault, AWS KMS)
- Store tokens in external secrets manager, reference by ID in DB
- **Pros:** Most secure; audit trail
- **Cons:** Infrastructure complexity; additional cost; overkill for MVP
- **Effort:** Large
- **Risk:** Low

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/models/user.py`
- `app/api/routes/calendar.py`
- `app/integrations/google_calendar.py`
- New migration needed to change column type

## Acceptance Criteria

- [ ] Google credentials are encrypted at rest in the database
- [ ] Encryption key is managed via environment variable
- [ ] Calendar functionality works correctly with encrypted storage
- [ ] Existing credentials are migrated (or users re-authenticate)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Identified by 3 agents |

## Resources

- cryptography.fernet documentation
