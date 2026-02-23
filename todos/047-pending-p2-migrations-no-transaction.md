---
status: pending
priority: p2
issue_id: "047"
tags: [code-review, database, safety, rust-v2]
dependencies: []
---

# Database Migrations Lack Transaction Wrapping

## Problem Statement
`migrate_v1()`, `migrate_v2()`, and `migrate_v3()` use `execute_batch` without wrapping in explicit transactions. A partial failure leaves the database in an inconsistent state (some tables created, version not bumped).

**Location:** `crates/mika-agent/src/db.rs` - migrate_v1, migrate_v2, migrate_v3

**Reported by:** pattern-recognition-specialist

## Proposed Solutions

### Option A: Wrap each migration in conn.transaction() (Recommended)
- **Effort:** Small
- **Risk:** None — SQLite transactions are well-supported

## Acceptance Criteria
- [ ] Each migration runs inside an explicit transaction
- [ ] Partial migration failure rolls back cleanly

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
