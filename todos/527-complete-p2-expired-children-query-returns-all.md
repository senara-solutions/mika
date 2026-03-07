---
status: complete
priority: p2
issue_id: 527
tags: [code-review, performance, task-engine]
dependencies: []
---

# get_expired_child_task_ids Returns ALL Expired Children, Not Just Newly Expired

## Problem Statement

`check_expired_siblings()` calls `get_expired_child_task_ids()` every 60 seconds, which returns ALL expired child tasks ever (until pruning). Most of these already had their parent claimed, so `try_complete_parent_on_sibling_done` returns `None` for them — but each check still executes a transaction with two queries.

**Severity:** P2 — O(E) redundant work per tick cycle where E = total expired children.

## Findings

- `crates/mika-agent/src/task_engine/engine.rs:224` — calls `get_expired_child_task_ids()` every 60 ticks
- `crates/mika-agent/src/db.rs:874` — `SELECT id FROM tasks WHERE ... status = 'expired' AND parent_task_id IS NOT NULL` (no parent status filter)

## Proposed Solutions

1. **Filter to only expired tasks whose parent is still pending**
   - Add a JOIN: `JOIN tasks p ON t.parent_task_id = p.id WHERE ... AND p.status = 'pending'`
   - Pros: Eliminates all redundant checks
   - Effort: Small
   - Risk: Low

2. **Have mark_tasks_expired return the IDs it just expired**
   - Modify `mark_tasks_expired` to return `Vec<String>` of affected IDs
   - Only check sibling completion for those specific IDs
   - Pros: No extra query needed
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Expired child check only processes actionable tasks (parent still pending)
