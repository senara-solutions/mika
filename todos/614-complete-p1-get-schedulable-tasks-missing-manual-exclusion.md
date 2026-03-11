---
status: complete
priority: p1
issue_id: 614
tags: [code-review, performance, correctness, task-engine]
dependencies: []
---

# get_schedulable_tasks does not exclude manual trigger_type

## Problem Statement

`get_schedulable_tasks()` in `db.rs` filters `WHERE trigger_type != 'callback'` but does not exclude `trigger_type = 'manual'`. Manual tasks have `next_fire_at = NULL`, so the task engine's 60-tick DB scan will load them, log a warning for each ("task has no next_fire_at"), and discard them. This creates unnecessary DB reads and log noise that grows with the number of work items.

Compare with `get_user_visible_tasks` which correctly uses `WHERE trigger_type NOT IN ('callback', 'manual')`.

## Findings

- **Source**: Pattern review agent, Performance review agent
- **Location**: `crates/mika-agent/src/db.rs` line ~1299
- **Evidence**: The query uses `trigger_type != 'callback'` instead of `NOT IN ('callback', 'manual')`

## Proposed Solutions

### Option A: Update the WHERE clause (Recommended)
Change `WHERE trigger_type != 'callback'` to `WHERE trigger_type NOT IN ('callback', 'manual')`.

- **Pros**: Simple, consistent with `get_user_visible_tasks`
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] `get_schedulable_tasks` excludes manual tasks
- [ ] No warning log spam for manual tasks during tick loop
- [ ] Existing tests pass
