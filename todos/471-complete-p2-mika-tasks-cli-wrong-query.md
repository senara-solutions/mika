---
status: pending
priority: p2
issue_id: "471"
tags: [code-review, correctness, cli, task-engine]
dependencies: []
---

# 471 · `mika tasks` CLI excludes recurring tasks — uses wrong query + dead filter

## Problem Statement

`mika tasks` calls `get_schedulable_tasks` (returns `status IN ('pending',
'recurring_active')`) then post-filters to only show `pending` and
`in_progress`. This means:

1. `recurring_active` heartbeat and reflection tasks are silently excluded — users cannot see them.
2. The `in_progress` filter is dead: `get_schedulable_tasks` never returns `in_progress` tasks.

A user running `mika tasks` sees "No pending tasks" even when heartbeat and reflection are actively scheduled.

## Findings

- **Location:** `crates/mika-cli/src/commands/tasks.rs:13–17`
- `get_schedulable_tasks` at `crates/mika-agent/src/db.rs:747` filters `status IN ('pending','recurring_active')`
- The CLI filter at line 16 excludes `recurring_active`

## Proposed Solutions

### Option A — Use `get_tasks_by_status` with correct statuses (recommended)
```rust
let tasks = db.get_tasks_by_status(vec![
    "pending".to_string(),
    "in_progress".to_string(),
    "recurring_active".to_string(),
]).await?;
// Remove the post-fetch filter
```

**Effort:** Trivial | **Risk:** Low

### Option B — Add a dedicated `get_active_tasks` method
Returns all non-terminal tasks (everything except completed/failed/cancelled/expired).
**Pros:** Semantically clear.
**Effort:** Small | **Risk:** Low

## Recommended Action

Option A for now.

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/tasks.rs`

## Acceptance Criteria

- [ ] Recurring tasks (heartbeat, reflection) appear in `mika tasks` output
- [ ] Display shows `status` column so user can distinguish `pending` from `recurring_active`
- [ ] `in_progress` filter removed or replaced with correct query

## Work Log

- 2026-03-06: Identified by code quality review agent (QUAL-6)
