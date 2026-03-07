---
status: complete
priority: p1
issue_id: "500"
tags: [code-review, security, database, isolation]
dependencies: []
---

# `cancel_task` / `get_task` / `update_task_completed` DB Methods Missing `agent_id` Scope Filter

## Problem Statement

Five DB methods that operate on individual tasks use only `WHERE id = ?1` with no `agent_id` predicate, unlike `get_schedulable_tasks` and `mark_tasks_expired` which correctly include `AND agent_id = ?`. In multi-agent single-DB deployments (which `agent_id` column exists to support), this allows cross-agent task manipulation.

## Findings

- **Source**: security-sentinel (F-1 High), architecture-strategist (F-2 High)
- **Location**: `crates/mika-agent/src/db.rs` — `get_task` (line 767), `cancel_task` (line 838), `update_task_completed` (line 811), `claim_and_fire_task`, `update_task_failed`

`cancel_task` SQL (db.rs:838):
```sql
UPDATE tasks SET status = 'cancelled', updated_at = unixepoch()
WHERE id = ?1 AND status NOT IN ('completed','failed','cancelled','expired')
```

`get_task` SQL (db.rs:767):
```sql
SELECT ... FROM tasks WHERE id = ?1
```

Neither filters by `agent_id`. Compare to `mark_tasks_expired` (line 847) and `get_schedulable_tasks` (line 774) which both include `AND agent_id = ?`.

Current production impact: SQLite databases are per-agent files, so cross-agent contamination does not occur today. Future risk: when multi-agent single-DB deployments are introduced (the schema already has `agent_id` column for exactly this purpose), these unscoped queries become exploitable. The `cancel_task` tool already runs in the context of a specific `ToolContext` with a specific `db.agent_id` — adding the filter is a one-line change per method.

Same issue applies to `mika ask --task-id` CLI path (ask.rs:43-72) which calls `get_task` without agent scoping.

## Proposed Solutions

### Option A: Add `AND agent_id = ?2` to all five methods (Recommended)

For each affected method, add the agent_id parameter and WHERE clause guard:

```sql
-- get_task
SELECT ... FROM tasks WHERE id = ?1 AND agent_id = ?2

-- cancel_task
UPDATE tasks SET status = 'cancelled', updated_at = unixepoch()
WHERE id = ?1 AND agent_id = ?2 AND status NOT IN (...)

-- update_task_completed
UPDATE tasks SET status = 'completed', result = ?1, completed_at = unixepoch()
WHERE id = ?2 AND agent_id = ?3 AND status IN ('pending', 'in_progress')
```

Pass `db.agent_id` as the additional parameter at each call site. The pattern is already established in `get_schedulable_tasks` and `mark_tasks_expired`.

- **Effort**: Small | **Risk**: Low (additive, straightforward)

### Option B: Accept current per-file isolation as sufficient

Document that the current single-file-per-agent model makes cross-agent contamination impossible today. Add a TODO comment noting that `agent_id` scoping is needed before multi-DB consolidation.

- **Effort**: Tiny | **Risk**: Technical debt (future vulnerability when DB model changes)

## Acceptance Criteria

- [ ] `get_task`, `cancel_task`, `update_task_completed`, `claim_and_fire_task`, `update_task_failed` all filter by `agent_id`
- [ ] All call sites pass `db.agent_id` as parameter
- [ ] Existing tests pass
- [ ] New test: `cancel_task` called with a task UUID that belongs to a different `agent_id` returns "not found" result

## Work Log

- 2026-03-06: Identified by security-sentinel and architecture-strategist reviews of feat/unified-task-engine
