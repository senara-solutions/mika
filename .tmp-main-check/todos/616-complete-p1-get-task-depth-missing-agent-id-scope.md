---
status: complete
priority: p1
issue_id: 616
tags: [code-review, security, authorization]
dependencies: []
---

# get_task_depth missing agent_id scope allows cross-agent parent linking

## Problem Statement

`get_task_depth()` queries `WHERE id = ?1` with no `agent_id` filter. An agent can supply any task UUID as `parent_task_id` in `create_work_item`, and the depth check will succeed if the task exists in any agent's task list. This breaks task tree isolation in multi-agent deployments.

## Findings

- **Source**: Security review agent
- **Location**: `crates/mika-agent/src/db.rs` line ~1631
- **Evidence**: Query is `SELECT depth FROM tasks WHERE id = ?1` — no agent_id filter

## Proposed Solutions

### Option A: Add agent_id to the query (Recommended)
```sql
SELECT depth FROM tasks WHERE id = ?1 AND agent_id = ?2
```
Update async wrapper to pass `agent_id`.

- **Pros**: Enforces task tree isolation
- **Cons**: Requires async_db signature change
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] `get_task_depth` filters by agent_id
- [ ] Cross-agent parent linking returns "not found"
- [ ] Existing parent/child tests still pass
