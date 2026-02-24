---
status: complete
priority: p3
issue_id: "061"
tags: [code-review, database, performance, rust-v2]
dependencies: []
---

# Unused idx_conv_created Index

## Problem Statement

The `idx_conv_created` index on `conversations(created_at)` is created in the schema but no query uses a WHERE or ORDER BY on `created_at` alone — `load_recent_messages()` orders by `id DESC`. The index adds write overhead with no read benefit.

**Location:** `crates/mika-agent/src/db.rs` — schema creation in `migrate_v4()`

**Reported by:** performance-oracle

## Proposed Solutions

Remove the index definition, or change `load_recent_messages()` to ORDER BY `created_at DESC` if that's the intended behavior.
- **Effort:** Tiny
- **Risk:** None

## Acceptance Criteria

- [ ] Index removed or a query uses it
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | Negligible impact — cosmetic cleanup |
