---
status: complete
priority: p2
issue_id: "556"
tags: [code-review, consistency]
dependencies: []
---

# Missing 'delivered' in cancel_recurring_task_by_label Terminal Status List

## Problem Statement

`cancel_recurring_task_by_label` at `db.rs:880` excludes terminal statuses but is missing `'delivered'`:
```sql
AND status NOT IN ('completed','failed','cancelled','expired')
```

All other terminal status guards were updated to include `'delivered'`:
- `cancel_task` (line 1014)
- `expire_timed_out_tasks` (line 1025)
- `try_complete_parent_on_sibling_done` (line 1188)

## Findings

- **Found by:** Pattern Recognition Specialist, Data Integrity Guardian (2/8 agents)

## Proposed Solutions

Add `'delivered'` to the exclusion list on line 880:
```sql
AND status NOT IN ('completed','failed','cancelled','expired','delivered')
```

**Effort:** Small (1 line)

## Acceptance Criteria

- [ ] `cancel_recurring_task_by_label` includes `'delivered'` in terminal status exclusion

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Consistency issue found by 2 agents |
| 2026-03-07 | Approved during triage | 1-line fix |
