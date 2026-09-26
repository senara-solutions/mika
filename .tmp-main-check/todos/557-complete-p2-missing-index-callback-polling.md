---
status: complete
priority: p2
issue_id: "557"
tags: [code-review, performance]
dependencies: []
---

# Missing Index for Callback Polling Query

## Problem Statement

`get_undelivered_callback_tasks` runs every ~5s and filters on `agent_id`, `trigger_type`, `action_type`, `status`, and `completed_at`. The only relevant index is `idx_tasks_agent_status(agent_id, status)`, which requires a linear scan of all completed tasks to apply remaining filters. At scale (thousands of completed tasks over 30 days), this becomes inefficient.

## Findings

- **Found by:** Performance Oracle (1/8 agents)
- **Location:** `crates/mika-agent/src/db.rs` — `get_undelivered_callback_tasks` query

## Proposed Solutions

Add a partial index:
```sql
CREATE INDEX idx_tasks_callback_delivery
    ON tasks(agent_id, completed_at)
    WHERE trigger_type = 'callback'
      AND action_type = 'resume_agent'
      AND status = 'completed';
```

Add to both `create_initial_schema()` and `migrate_v2()`.

**Effort:** Small
**Risk:** Low — partial index is tiny (only matching rows indexed)

## Acceptance Criteria

- [ ] Partial index exists in schema
- [ ] Callback polling query uses index scan (verifiable via EXPLAIN QUERY PLAN)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Current impact negligible, but grows linearly |
| 2026-03-07 | Approved during triage | Add partial index to both schema and migration |
