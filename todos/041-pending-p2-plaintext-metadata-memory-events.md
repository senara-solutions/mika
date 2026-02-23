---
status: pending
priority: p2
issue_id: "041"
tags: [code-review, security, database, rust-v2]
dependencies: []
---

# Plaintext Metadata Leaks in memory_events Table

## Problem Statement
The `target_key` column in `memory_events` stores plaintext identifiers like `person:Alice Chen` and `commitment:Review Q4 budget`. This leaks PII metadata alongside encrypted before/after values, undermining the encryption-at-rest model.

**Location:** `crates/mika-agent/src/tools/store_fact.rs` (lines 104, 138, 172, 197) and `crates/mika-agent/src/tools/update_core_memory.rs`

**Reported by:** security-sentinel

## Findings
- `target_key` is stored as plaintext TEXT column
- Values like `person:Alice Chen`, `commitment:Review Q4 budget`, `preference:meeting_time` contain user PII
- The `before_value_encrypted` and `after_value_encrypted` columns are properly encrypted
- An attacker with SQLite access can enumerate all people and commitments from target_key alone

## Proposed Solutions

### Option A: Encrypt target_key (Recommended)
Encrypt `target_key` the same way before/after values are encrypted.
- **Pros:** Consistent encryption model
- **Cons:** Cannot query by target_key anymore (but currently no query uses it)
- **Effort:** Small
- **Risk:** Low

### Option B: Use HMAC hash for target_key + encrypted variant
Store both `target_key_hash` (HMAC) and `target_key_encrypted` (AES).
- **Pros:** Allows future lookup by target while keeping it encrypted
- **Cons:** More complex schema change
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria
- [ ] target_key does not contain plaintext PII in the database
- [ ] Audit log still readable when decrypted
- [ ] Existing tests updated

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
