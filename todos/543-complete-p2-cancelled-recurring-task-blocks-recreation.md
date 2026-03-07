---
status: complete
priority: p2
issue_id: "543"
tags: [code-review, data-integrity, task-engine]
dependencies: []
---

# Cancelled recurring task blocks re-creation

## Problem Statement

When a recurring task is cancelled (e.g., reflection disabled in identity.toml), the partial unique index `idx_tasks_unique_recurring ON tasks(agent_id, label) WHERE trigger_type = 'recurring'` still covers the cancelled row. On the next startup, if reflection is re-enabled, `create_recurring_task_if_absent` uses `INSERT OR IGNORE` which silently fails because the cancelled row still satisfies the unique index. The `get_recurring_task_cron` check filters `status IN ('recurring_active', 'pending', 'in_progress')` so it doesn't find the cancelled row either. Result: the task can never be re-registered without manual DB intervention.

## Findings

- **Source:** Data integrity review agent
- **Location:** `crates/mika-agent/src/db.rs` lines 575-577 (index), 715-735 (`create_recurring_task_if_absent`), 767-775 (`cancel_recurring_task_by_label`)
- **Evidence:** The partial unique index does not exclude cancelled/expired/failed statuses. The INSERT OR IGNORE respects the index regardless of row status.

## Proposed Solutions

### Option A: Exclude terminal statuses from the unique index
- **Approach:** Change the partial unique index to `WHERE trigger_type = 'recurring' AND status NOT IN ('cancelled', 'failed', 'expired')`
- **Pros:** Clean semantic fix, allows re-creation after cancellation
- **Cons:** Requires schema change (DROP + CREATE INDEX), may need careful migration
- **Effort:** Small
- **Risk:** Low

### Option B: DELETE instead of cancel for recurring tasks
- **Approach:** Change `cancel_recurring_task_by_label` to DELETE the row instead of setting status to 'cancelled'
- **Pros:** Simplest fix, no schema change needed
- **Cons:** Loses audit trail of cancelled tasks
- **Effort:** Small
- **Risk:** Low

### Option C: Reactivate cancelled tasks instead of inserting new ones
- **Approach:** In `ensure_recurring_task`, after `create_recurring_task_if_absent` returns None, check for a cancelled task and UPDATE its status back to 'recurring_active' with the new cron
- **Pros:** No schema change, preserves task history
- **Cons:** More code complexity
- **Effort:** Medium
- **Risk:** Low

## Recommended Action

(To be filled during triage)

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/task_engine/mod.rs`
- **Affected components:** Task engine, recurring task lifecycle

## Acceptance Criteria

- [ ] Disabling reflection, restarting, re-enabling reflection, and restarting again results in a working reflection task
- [ ] Test covers the cancel-then-re-register lifecycle

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Latent interaction between cancel + partial unique index |

## Resources

- PR branch: `feat/unified-task-engine`
