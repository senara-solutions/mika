---
status: complete
priority: p1
issue_id: 525
tags: [code-review, performance, database]
dependencies: []
---

# Missing Database Indexes on parent_task_id and created_by_session

## Problem Statement

Three new query patterns filter on `parent_task_id` and one uses `LIKE` on `created_by_session`, but neither column has an index. This causes full table scans on the `tasks` table for every sibling completion check, child task lookup, and grandchild detection.

These queries run:
- After every task completion (HTTP handler and tick loop)
- Every 60 seconds for expiry checks
- On every team task tree creation

**Severity:** P1 — Grows linearly with task count, O(N^2) in expiry check loop.

## Findings

- `try_complete_parent_on_sibling_done` — `WHERE parent_task_id = ?1 AND agent_id = ?2`
- `get_child_tasks` — `WHERE parent_task_id = ?1 AND agent_id = ?2`
- `get_expired_child_task_ids` — `WHERE ... AND parent_task_id IS NOT NULL`
- `count_pending_callback_tasks_by_session_prefix` — `WHERE ... AND created_by_session LIKE ?2`
- Existing indexes: `(agent_id, status)`, `(agent_id, next_fire_at)` — no coverage for parent_task_id or created_by_session

## Proposed Solutions

1. **Add composite indexes in migrate_v1()**
   - `CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;`
   - `CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;`
   - Pros: Eliminates all table scans, partial indexes minimize storage overhead
   - Cons: Requires DB reset (v1 migration)
   - Effort: Small (2 lines DDL)
   - Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs` (schema in `migrate_v1()`)
- **Components:** Database schema, task queries

## Acceptance Criteria

- [ ] `parent_task_id` composite index added to schema
- [ ] `created_by_session` index added to schema
- [ ] All sibling/child queries use indexes (verified via EXPLAIN QUERY PLAN)
