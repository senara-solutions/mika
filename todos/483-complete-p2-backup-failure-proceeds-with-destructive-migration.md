---
status: complete
priority: p2
issue_id: "483"
tags: [code-review, database, data-integrity]
dependencies: []
---

# Backup Failure Does Not Abort Schema Migration Before Destructive Drop-All

## Problem Statement

In `Database::open()`, before calling `migrate_v1()` (which drops all tables), the code
attempts to copy the existing DB to a backup path using `std::fs::copy`. If the copy fails
(e.g., disk full, permissions error), the code **logs a warning and proceeds** with the
destructive migration. This means a user upgrading from a pre-v1 schema on a full disk will
lose all their data without a backup being created. The backup is the only safety net for the
clean-slate migration.

## Findings

- **Source**: architecture-strategist review
- **Location**: `crates/mika-agent/src/db.rs:241–256`
- `match std::fs::copy(...)` Err arm logs warning and falls through to `db.migrate()`
- `migrate_v1()` begins with `DROP TABLE IF EXISTS ...` for every table
- Only affects users upgrading from pre-v1 schemas; new installs (version = 0) go directly
  to migrate without backup

## Proposed Solutions

### Option A: Return Err on backup failure (Recommended)
```rust
match std::fs::copy(path, &backup_path) {
    Ok(_) => { /* proceed */ }
    Err(e) => {
        return Err(anyhow::anyhow!(
            "cannot auto-backup DB before migration: {e}. Aborting to protect data. \
             Free disk space or manually backup {} before retrying.",
            path.display()
        ));
    }
}
```
- **Pros**: Prevents data loss, gives user actionable error message
- **Cons**: User must manually resolve before migration can proceed
- **Effort**: Tiny | **Risk**: None

### Option B: Add force-migrate flag
Allow overriding the backup failure with an env var (`MIKA_FORCE_MIGRATE=1`) that allows
proceeding without a backup.
- **Pros**: Escape hatch for recovery scenarios
- **Cons**: More complex, potential misuse
- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] Backup failure returns `Err` (does not proceed with migration)
- [ ] Error message is actionable (tells user what failed and how to resolve)
- [ ] Behavior on fresh installs (version = 0) is unchanged (no backup needed)
- [ ] Unit test for backup failure behavior added

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
