---
status: complete
priority: p1
issue_id: 563
tags:
  - code-review
  - security
  - database
  - correctness
dependencies: []
---

# is_unique_violation() catches all constraint types, not just UNIQUE

## Problem Statement

`is_unique_violation()` in `db.rs` checks `ErrorCode::ConstraintViolation`, which maps to SQLite error code `SQLITE_CONSTRAINT` (19). This covers ALL constraint types: UNIQUE, NOT NULL, CHECK, FOREIGN KEY, and PRIMARY KEY. The function name implies it only catches UNIQUE violations, but it would silently swallow any constraint error as "duplicate already exists."

If a future schema change adds a CHECK constraint on the `tasks` or `events` table, or if a bug introduces a NOT NULL/FK violation in the INSERT path, the error would be masked as a successful "already exists" response instead of surfacing the real failure.

## Findings

- **File:** `crates/mika-agent/src/db.rs:24-30`
- **Flagged by:** Security Sentinel, Architecture Strategist, Performance Oracle, Code Simplicity Reviewer (4/6 agents)
- `rusqlite::Error::SqliteFailure` has an `extended_code` field that distinguishes `SQLITE_CONSTRAINT_UNIQUE` (2067) from other constraint types
- Current risk is low (tool inputs are validated before DB), but this is a maintenance hazard

## Proposed Solutions

### Option A: Check extended_code for SQLITE_CONSTRAINT_UNIQUE (Recommended)

```rust
pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(e, _)) = err.downcast_ref::<rusqlite::Error>() {
        e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    } else {
        false
    }
}
```

- **Pros:** Precise, zero cost, prevents masking other constraint errors
- **Cons:** None
- **Effort:** Small (one-line change)
- **Risk:** None

### Option B: Rename to `is_constraint_violation` (Not recommended)

- **Pros:** Honest about what it checks
- **Cons:** Doesn't fix the masking problem — just relabels it
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Option A. One-line change with no downsides.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`
- **Affected components:** `is_unique_violation()` helper, called from `create_reminder.rs` and `store_fact.rs`

## Acceptance Criteria

- [ ] `is_unique_violation()` checks `e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE`
- [ ] Existing tests still pass (constraint violations from the new indexes produce the UNIQUE extended code)
- [ ] Add a test: verify a non-UNIQUE constraint error is NOT caught by `is_unique_violation()`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Found during code review | Consensus across 4/6 review agents |

## Resources

- [SQLite Result Codes](https://www.sqlite.org/rescode.html#constraint_unique)
- `rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE` = 2067
