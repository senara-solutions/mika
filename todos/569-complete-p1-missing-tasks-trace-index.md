---
status: complete
priority: p1
issue_id: "569"
tags: [code-review, performance, observability, database, index]
dependencies: []
---

# Missing partial index on tasks.created_trace_id

## Problem Statement

The `messages` and `audit_events` tables have partial indexes on `trace_id` (`WHERE trace_id IS NOT NULL`), but the `tasks` table has no corresponding index on `created_trace_id`. The `unified_timeline` VIEW includes tasks, so querying `WHERE trace_id = ?` forces a full table scan on the tasks branch while the other two branches use index lookups.

## Findings

- **Source:** Performance Oracle
- **File:** `crates/mika-agent/src/db.rs` — both `migrate_v1()` and `migrate_v4_to_v5()`
- **Evidence:** `idx_msg_trace` and `idx_audit_trace` exist but no `idx_tasks_trace`
- **Impact:** O(n) full table scan on tasks table for every `unified_timeline WHERE trace_id = ?` query

## Proposed Solutions

### Option A: Add partial index (Recommended)
```sql
CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
```
Add to both `migrate_v1()` (clean-slate) and `migrate_v4_to_v5()` (step 5).

- **Pros:** Consistent with existing pattern, O(log n) lookup
- **Cons:** Small write overhead per INSERT (~100ns)
- **Effort:** Small (10 min)
- **Risk:** None

## Acceptance Criteria

- [ ] Index exists in both migration paths
- [ ] `unified_timeline WHERE trace_id = ?` uses index for all three branches

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Consistency gap in index coverage |
