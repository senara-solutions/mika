---
status: pending
priority: p3
issue_id: "735"
tags: [code-review, quality, db]
---

# Extract `task_from_row` helper to DRY up Task struct construction

## Problem Statement

The `Task` struct is constructed from rusqlite `Row` objects in multiple places across `db.rs`, each duplicating the same 29-field mapping. Adding `find_active_work_item_by_ref_url` and `find_active_work_item_by_label` in #303 added two more copies.

## Findings

- `find_active_work_item_by_ref_url` (line ~2932) — 29-field mapping
- `find_active_work_item_by_label` (line ~2992) — identical 29-field mapping
- `get_task` (line ~2609) — same mapping (pre-existing)
- Adding a new column to `Task` requires updating all sites

## Proposed Solutions

### Option A: Extract a `task_from_row(r: &Row) -> rusqlite::Result<Task>` helper

```rust
fn task_from_row(r: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        // ... all 29 fields
    })
}
```

- **Pros:** Simple, ~60 LOC reduction, single maintenance point
- **Cons:** Assumes consistent column ordering across all queries
- **Effort:** Small
- **Risk:** Low — compile-time type checking catches mismatches

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria

- [ ] Single `task_from_row` helper replaces all duplicated mappings
- [ ] All existing tests pass
