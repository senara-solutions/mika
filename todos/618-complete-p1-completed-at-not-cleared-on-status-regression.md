---
status: complete
priority: p1
issue_id: 618
tags: [code-review, correctness, data-integrity]
dependencies: []
---

# completed_at not cleared when status regresses from completed

## Problem Statement

`update_manual_task_status` sets `completed_at = unixepoch()` when transitioning to `completed`, but does not clear it when transitioning away from `completed` (e.g., back to `in_progress`). This leaves stale `completed_at` timestamps that misrepresent the task's actual completion history.

## Findings

- **Source**: Architecture review agent
- **Location**: `crates/mika-agent/src/db.rs` — `update_manual_task_status` method
- **Evidence**: The UPDATE statement uses `CASE WHEN ?2 = 'completed' THEN unixepoch() ELSE completed_at END` — the ELSE branch preserves old value instead of clearing

## Proposed Solutions

### Option A: Clear completed_at on non-completed transitions (Recommended)
```sql
completed_at = CASE WHEN ?2 = 'completed' THEN unixepoch() ELSE NULL END
```

- **Pros**: Clean data semantics
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Transitioning away from `completed` sets `completed_at = NULL`
- [ ] Test: complete → in_progress → verify completed_at is NULL
