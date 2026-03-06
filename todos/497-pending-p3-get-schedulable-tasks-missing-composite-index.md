---
status: pending
priority: p3
issue_id: "497"
tags: [code-review, performance, database]
dependencies: []
---

# get_schedulable_tasks Missing Composite Partial Index for Efficient Sorting

## Problem Statement

`get_schedulable_tasks` queries:
```sql
SELECT ... FROM tasks
WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
ORDER BY next_fire_at ASC NULLS LAST
```

The existing `idx_tasks_agent_status` covers the `WHERE` clause but the `ORDER BY next_fire_at`
requires a separate sort step. The partial index `idx_tasks_next_fire` doesn't include `agent_id`,
so it isn't used for this query. A composite partial index would allow SQLite to satisfy both
the filter and sort in one index scan.

## Findings

- **Source**: performance-oracle review
- **Location**: `crates/mika-agent/src/db.rs:415–417` (existing indexes), query at line 779–781
- At current volumes (2–10 tasks per agent): unmeasurable impact
- At high volumes (1000+ tasks): eliminates O(n log n) sort step

## Proposed Solutions

### Option A: Add composite partial index (Recommended)
```sql
CREATE INDEX idx_tasks_schedulable
    ON tasks(agent_id, next_fire_at ASC)
    WHERE status IN ('pending', 'recurring_active');
```
Add to schema v1 definition in `db.rs`. This is a non-breaking schema change (additional index only).
- **Effort**: Tiny | **Risk**: None

### Option B: Accept current design
At current task counts (always tiny), the sort overhead is negligible. Document as future
optimization.
- **Effort**: None | **Risk**: None

## Acceptance Criteria

- [ ] A composite partial index on `(agent_id, next_fire_at) WHERE status IN (...)` exists in the schema
- [ ] SQLite EXPLAIN QUERY PLAN confirms the index is used for `get_schedulable_tasks`
- [ ] Existing tests pass

## Work Log

- 2026-03-06: Identified by performance-oracle review of feat/unified-task-engine
