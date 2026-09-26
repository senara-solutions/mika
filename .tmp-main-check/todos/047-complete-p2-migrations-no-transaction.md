---
status: complete
priority: p2
issue_id: "047"
tags: [code-review, database, safety, rust-v2]
dependencies: []
---

# Database Migrations Lack Transaction Wrapping

## Problem Statement
`migrate_v1()`, `migrate_v2()`, `migrate_v3()`, and `migrate_v4()` use `execute_batch` without wrapping in explicit transactions. A partial failure leaves the database in an inconsistent state (some tables created, version not bumped). `migrate_v4()` is especially risky — it DROPs all tables and recreates them.

**Location:** `crates/mika-agent/src/db.rs` — migrate_v1 through migrate_v4

**Reported by:** pattern-recognition-specialist, performance-oracle, learnings-researcher, architecture-strategist

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
| 2026-02-24 | Re-confirmed in encryption-strip review — now includes migrate_v4() which DROPs all tables | 4 agents flagged this |
