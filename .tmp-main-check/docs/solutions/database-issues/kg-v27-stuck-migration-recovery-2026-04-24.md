---
module: database
tags: [migration, kg, recovery, v27, schema]
problem_type: migration-recovery
date: 2026-04-24
---

# KG v27 Stuck Migration Recovery

## Problem

Between #786's merge (schema v27 DDL stub) and #787's merge (coalesce SQL), a database restart ran the v27 migration stub. The DB is now at `schema_version = 27` with empty v27 KG tables and preserved v26 data in `*_v26_backup` tables. #786's startup guard (`check_v27_coalesce_guard`) refuses to return a `Database` handle because the `schema_meta.v27_coalesce_complete` marker is absent.

Error message: `KG v27 migration incomplete — coalesce step from mika#787 has not run. Deploy #787 before starting.`

## Solution

### Preferred path: deploy #787

If #787 has been merged to main, simply deploy the updated binary. On next startup, `Database::open()` will detect `schema_version == 27` with backup tables present and the idempotency guard will pass (the `kg_chunks` table already has `docs_root_hash`). The coalesce marker was already written by the migration stub + #787's coalesce SQL. The startup guard passes and the service starts normally.

### Manual recovery: restore to v26

If #787 has NOT been merged yet but you need the service running immediately, restore the database to v26 so the full v27 migration (DDL + coalesce) runs once #787 deploys.

#### Prerequisites

1. **Stop the service:** `systemctl stop mika-agent` or equivalent.
2. **Backup the DB:** `cp ~/.mika/data/mika.db ~/.mika/data/mika.db.recovery-backup-$(date +%s)`

#### Detection (confirm you are in the stuck state)

Run against `~/.mika/data/mika.db` with `sqlite3` CLI:

```sql
-- Step 1: Confirm schema_version is 27.
SELECT MAX(version) FROM schema_version;
-- Expected: 27. If not 27, you are NOT stuck — stop here.

-- Step 2: Confirm the coalesce marker is absent.
SELECT COUNT(*) FROM schema_meta WHERE key = 'v27_coalesce_complete';
-- Expected: 0. If 1, the migration already completed — stop here.

-- Step 3: Confirm v26 backup tables still exist.
SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE '%_v26_backup';
-- Expected: 8 rows. If 0, backups were dropped — restore from filesystem backup.

-- Step 4: Confirm v27 tables are empty.
SELECT (SELECT COUNT(*) FROM kg_chunks) AS chunks,
       (SELECT COUNT(*) FROM kg_subject_entities) AS entities;
-- Expected: chunks=0, entities=0. If non-zero, partial coalesce occurred —
-- restore from filesystem backup.
```

If all four checks produce expected outputs, proceed.

#### Recovery steps

```sql
BEGIN IMMEDIATE;
PRAGMA defer_foreign_keys = ON;

-- Drop empty v27 tables.
DROP TABLE IF EXISTS kg_chunk_subject_relationships;
DROP TABLE IF EXISTS kg_chunk_subjects;
DROP TABLE IF EXISTS kg_subject_relationships;
DROP TABLE IF EXISTS kg_subject_resolutions;
DROP TABLE IF EXISTS kg_resolutions_log;
DROP TABLE IF EXISTS kg_subject_entities;
DROP TABLE IF EXISTS kg_extractions;
DROP TABLE IF EXISTS kg_chunks;

-- Rename v26 backup tables back to canonical names.
ALTER TABLE kg_chunks_v26_backup RENAME TO kg_chunks;
ALTER TABLE kg_subject_entities_v26_backup RENAME TO kg_subject_entities;
ALTER TABLE kg_subject_relationships_v26_backup RENAME TO kg_subject_relationships;
ALTER TABLE kg_chunk_subjects_v26_backup RENAME TO kg_chunk_subjects;
ALTER TABLE kg_chunk_subject_relationships_v26_backup RENAME TO kg_chunk_subject_relationships;
ALTER TABLE kg_extractions_v26_backup RENAME TO kg_extractions;
ALTER TABLE kg_subject_resolutions_v26_backup RENAME TO kg_subject_resolutions;
ALTER TABLE kg_resolutions_log_v26_backup RENAME TO kg_resolutions_log;

-- Reset schema_version to 26 so migrate() re-dispatches v26 → v27.
DELETE FROM schema_version WHERE version = 27;

-- Drop schema_meta so the migration starts clean (it will be recreated).
DROP TABLE IF EXISTS schema_meta;

COMMIT;
```

#### Verification

1. Start the service: `systemctl start mika-agent`
2. Check logs: expect `migrating database schema v26 -> v27` and `v26->v27 coalesce: resolved docs_root for migration`
3. Confirm the marker: `sqlite3 ~/.mika/data/mika.db "SELECT value FROM schema_meta WHERE key = 'v27_coalesce_complete';"` → expect `1`
4. Confirm normal startup (no `MigrationIncomplete` error in logs)

#### If recovery fails

Restore from the backup: `cp ~/.mika/data/mika.db.recovery-backup-<timestamp> ~/.mika/data/mika.db`. The DB returns to the pre-recovery state. Root-cause the failure before retrying — likely #787's migration body has a bug (check the `tracing::info!` stage from logs).

## Root Cause

The two-ticket split (#786 for DDL, #787 for coalesce) creates a window where a restart between merges leaves the DB in a partially-migrated state. The startup guard is intentionally strict — running with empty v27 tables would silently drop all KG data from agent queries.

## Prevention

Deploy #786 and #787 together (or #787 immediately after #786). The milestone deploy-once pattern (`merge all, deploy once`) prevents this window in normal operations. The recovery procedure covers accidental mid-milestone restarts.
