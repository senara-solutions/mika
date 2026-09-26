---
status: complete
priority: p1
issue_id: "158"
tags: [code-review, security, data-integrity]
---

# Fix Nullable pairing_expires_at Allowing Immortal Tokens

## Problem Statement
The `pairing_expires_at` column is nullable, and the pairing query treats NULL as "never expires" via `(pairing_expires_at IS NULL OR pairing_expires_at > now())`. A provisioning bug that omits expiry creates tokens that live forever, violating the stated 24h security design.

## Findings
- **Data integrity guardian**: HIGH severity — if provisioning forgets to set expiry, token is permanently valid

## Proposed Solutions

### Option A: Remove IS NULL bypass from pairing query (Recommended)
Change the WHERE clause to:
```sql
AND pairing_expires_at > now()
```
NULL expiry means "not paireable" rather than "pair forever". Safest default.
- Pros: Defense-in-depth; provisioning bug = inert token (not infinite token)
- Cons: Provisioning MUST set expiry (correct behavior)
- Effort: Small (5 min)
- Risk: None — no production data exists yet

### Option B: Make column NOT NULL
- Pros: Schema-level enforcement
- Cons: Requires migration change; NULL is useful for "no active pairing"
- Effort: Small
- Risk: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (line 232)

## Acceptance Criteria
- [ ] NULL `pairing_expires_at` does NOT pass the pairing check
- [ ] Tokens without explicit expiry are rejected
- [ ] Test added for NULL expiry behavior

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
