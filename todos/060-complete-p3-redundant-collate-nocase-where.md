---
status: complete
priority: p3
issue_id: "060"
tags: [code-review, database, quality, rust-v2]
dependencies: []
---

# Redundant COLLATE NOCASE in WHERE Clauses

## Problem Statement

Several queries include explicit `COLLATE NOCASE` in WHERE clauses (e.g., `WHERE canonical_name = ?1 COLLATE NOCASE`) when the column already has `COLLATE NOCASE` defined in the CREATE TABLE. SQLite applies the column's collation automatically — the WHERE clause annotation is redundant.

**Location:** `crates/mika-agent/src/db.rs` — `get_person()`, `get_preference()`, and related queries

**Reported by:** performance-oracle

## Proposed Solutions

Remove `COLLATE NOCASE` from WHERE clauses where the column definition already specifies it.
- **Effort:** Tiny
- **Risk:** None

## Acceptance Criteria

- [ ] No redundant COLLATE NOCASE in WHERE clauses
- [ ] Case-insensitive tests still pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | Cosmetic — no behavioral change |
