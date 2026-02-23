---
status: complete
priority: p2
issue_id: "033"
tags: [code-review, security, privacy, rust-v2]
dependencies: []
---

# Plaintext PII Columns in Encrypted SQLite Database

## Problem Statement

While sensitive content fields are encrypted, several columns containing PII or sensitive metadata are stored in plaintext:
- `people.canonical_name` — full names of contacts
- `people.relationship` — relationship types
- `commitments.status`, `commitments.due_date`
- `preferences.category`
- `events.event_date`, `events.context`

An attacker with file access can determine who the user talks about, their relationships, commitment deadlines, and daily patterns without breaking AES encryption.

**Why it matters:** Partial PII exposure undermines the encryption promise. Names and relationships are sensitive executive data.

## Findings

- **Source:** Security Sentinel (H3)
- **Location:** `crates/mika-agent/src/db.rs` migration v1 schema

## Proposed Solutions

### Option A: Encrypt PII columns, keep metadata plain (Recommended)
- Encrypt `canonical_name` and `relationship` in people table
- Encrypt `category` in preferences
- Keep timestamps and status unencrypted (needed for indexes/queries)
- Add encrypted lookup via HMAC hash for name-based queries
- **Pros:** Protects the most sensitive metadata
- **Cons:** Cannot do SQL queries on encrypted columns (need HMAC index)
- **Effort:** Medium
- **Risk:** Medium (changes query patterns)

### Option B: Full database encryption via SQLCipher
- Enable `bundled-sqlcipher` feature in rusqlite
- Encrypts entire database file including metadata
- **Pros:** Complete protection, simplest conceptually
- **Cons:** Unknown compatibility with sqlite-vec, performance overhead
- **Effort:** Medium
- **Risk:** High (sqlite-vec compatibility untested per plan)

## Acceptance Criteria

- [ ] Contact names not readable in raw SQLite file
- [ ] Relationship types not readable in raw SQLite file
- [ ] Queries still work for name-based lookups (via HMAC or other mechanism)
