---
status: complete
priority: p1
issue_id: 564
tags:
  - code-review
  - database
  - migration
  - correctness
dependencies: []
---

# New UNIQUE indexes not applied to existing v3 databases

## Problem Statement

The two new UNIQUE partial indexes (`idx_tasks_unique_reminder`, `idx_events_unique_description`) are added inside `migrate_v1()` — the clean-slate schema creation function. The schema version remains at 3. Existing databases already at version 3 will never execute these `CREATE UNIQUE INDEX` statements.

Any user with an existing Mika installation will not get duplicate protection until they reset their database.

## Findings

- **File:** `crates/mika-agent/src/db.rs`, lines 624-634 (indexes in `migrate_v1()`)
- **Flagged by:** Architecture Strategist
- The migration system checks `PRAGMA user_version` and only runs the appropriate migration path
- Since version stays at 3, the v1→v3 and v2→v3 migration paths (which call `migrate_v1()`) will not run for existing v3 databases
- The indexes use `IF NOT EXISTS`, making them safe to run idempotently

## Proposed Solutions

### Option A: Bump to schema v4 with migration (Recommended)

Add a `migrate_v3_to_v4()` function that creates the two indexes and updates `PRAGMA user_version` to 4. Update `CURRENT_SCHEMA_VERSION` to 4.

- **Pros:** Clean migration path, follows existing pattern
- **Cons:** Bumps schema version
- **Effort:** Small
- **Risk:** Low (indexes use `IF NOT EXISTS`)

### Option B: Run index creation on every startup

Add an `ensure_indexes()` function that runs the `CREATE UNIQUE INDEX IF NOT EXISTS` statements on every database open, regardless of version.

- **Pros:** No schema version change needed
- **Cons:** Runs SQL on every startup (minor), breaks the version-based migration model
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Option A — consistent with the existing migration architecture.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`
- **Schema change:** v3 → v4
- **Migration SQL:** Two `CREATE UNIQUE INDEX IF NOT EXISTS` statements

## Acceptance Criteria

- [ ] `CURRENT_SCHEMA_VERSION` bumped to 4
- [ ] `migrate_v3_to_v4()` creates both indexes
- [ ] `ensure_schema()` dispatches v3 → v4 migration
- [ ] Existing v3 databases get the indexes on next startup
- [ ] New databases still get indexes (via `migrate_v1()`)
- [ ] `cargo test` passes

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Found during architecture review | Indexes in migrate_v1() only affect new databases |

## Resources

- Existing migration pattern: `migrate_v1_to_v2()`, `migrate_v2_to_v3()` in db.rs
