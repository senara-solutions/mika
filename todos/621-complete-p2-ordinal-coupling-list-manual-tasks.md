---
status: complete
priority: p2
issue_id: 621
tags: [code-review, fragility, maintenance]
dependencies: []
---

# Hardcoded ordinal 26 in list_manual_tasks for child_count column

## Problem Statement

`list_manual_tasks` reads `child_count` at hardcoded column ordinal 26 (one past the last TASK_COLUMNS index at 25). Any future column addition to TASK_COLUMNS that doesn't also update this ordinal will silently read the wrong value. Additionally, the `TASK_COLUMNS.split(", ").map(|c| format!("t.{c}"))` pattern is fragile.

## Findings

- **Source**: Architecture review agent, Simplicity review agent
- **Location**: `crates/mika-agent/src/db.rs` — `list_manual_tasks`

## Proposed Solutions

### Option A: Use static NULL-check SQL pattern (Recommended)
Replace dynamic SQL with parameterized NULL checks:
```sql
WHERE (?2 IS NULL OR t.status = ?2) AND (?3 IS NULL OR t.source = ?3)
```
Use a TASK_COLUMNS_ALIASED constant or inline the prefixed columns.

- **Effort**: Medium
- **Risk**: Low

## Acceptance Criteria

- [ ] No hardcoded column ordinal
- [ ] No runtime string splitting of TASK_COLUMNS
- [ ] All existing tests pass
