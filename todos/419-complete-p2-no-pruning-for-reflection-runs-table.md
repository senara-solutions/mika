---
status: complete
priority: p2
issue_id: "419"
tags: [code-review, database, reflection, housekeeping]
dependencies: []
---

# No Cleanup/Pruning for reflection_runs Table

## Problem Statement

The `reflection_runs` table has no cleanup mechanism. Other similar tables have pruning:
- `heartbeat_sends`: pruned to 7 days in `recover()`
- `memory_events`: compacted after 90 days

Growth is ~365 rows/year (negligible), but this breaks the project's defensive housekeeping pattern.

## Findings

- **Data integrity guardian**: "Low practical risk but violates the project's defensive housekeeping pattern"

## Proposed Solutions

### Option A: Add pruning in recover() (Recommended)
Add `prune_old_reflection_runs(90)` call in `recover()`, keeping 90 days for consistency.
- **Effort**: Small
- **Risk**: Low

## Technical Details

- **Affected files**: `crates/mika-agent/src/db.rs` (add prune query), `scheduler.rs` (add prune call in recover())

## Acceptance Criteria

- [ ] Reflection runs older than 90 days are pruned on startup
- [ ] Prune function has tests
