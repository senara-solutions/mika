---
status: complete
priority: p2
issue_id: 625
tags: [code-review, performance, indexing]
dependencies: []
---

# list_active_work_items lacks covering partial index

## Problem Statement

`list_active_work_items()` runs on every silent agent invocation. The query filters on `agent_id`, `trigger_type = 'manual'`, and `status IN ('pending', 'in_progress', 'blocked')`. No existing index covers this combination efficiently.

## Findings

- **Source**: Performance review agent

## Proposed Solutions

### Option A: Add partial index (Recommended)
```sql
CREATE INDEX idx_tasks_manual_active ON tasks(agent_id, created_at DESC)
  WHERE trigger_type = 'manual' AND status IN ('pending', 'in_progress', 'blocked');
```
Add to both clean-slate schema and v7→v8 migration.

- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Index added to schema and migration
- [ ] list_active_work_items uses index scan
