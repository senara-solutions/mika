---
status: pending
priority: p1
issue_id: "462"
tags: [code-review, correctness, database, multi-agent]
dependencies: []
---

# 462 · `mark_tasks_expired` lacks `agent_id` filter — cross-agent contamination

## Problem Statement

`mark_tasks_expired` runs at startup via `startup_recovery` and marks any
task whose `timeout_at < now` as `expired`, regardless of which agent owns
it. In a multi-agent deployment where multiple agents share the same SQLite
DB, this function would expire another agent's in-flight tasks. Every other
task DB method was updated to scope by `agent_id` in this branch — this one
was missed.

## Findings

- **Location:** `crates/mika-agent/src/db.rs:804–812`
- The SQL does not include `WHERE agent_id = ?`
- All sibling methods (`get_schedulable_tasks`, `get_tasks_by_status`, etc.) correctly filter by `agent_id`
- `AsyncDatabase::mark_tasks_expired` passes `agent_id` into the closure via closure capture, so it is available — just not forwarded to the SQL

## Proposed Solutions

### Option A — Add `agent_id` to the WHERE clause (recommended)
```sql
UPDATE tasks SET status = 'expired', updated_at = unixepoch()
WHERE agent_id = ?2
  AND timeout_at IS NOT NULL AND timeout_at < ?1
  AND status NOT IN ('completed','failed','cancelled','expired')
```
Update signature: `fn mark_tasks_expired(&self, now: i64, agent_id: &str) -> Result<u64>`.

**Effort:** Small | **Risk:** Low

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`

## Acceptance Criteria

- [ ] `mark_tasks_expired` SQL includes `AND agent_id = ?`
- [ ] Signature updated to accept `agent_id`
- [ ] `AsyncDatabase` wrapper passes `agent_id` via closure
- [ ] Test: two agents each have an expiring task; running recovery for agent A expires only agent A's task

## Work Log

- 2026-03-06: Identified by architecture review agent (ARCH-3)
