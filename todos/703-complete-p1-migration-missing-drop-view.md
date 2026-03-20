---
status: pending
priority: p1
issue_id: 703
tags: [code-review, database]
dependencies: []
---

# Migration v12→v13 Missing DROP VIEW Before Table Rebuild

## Problem Statement

Migration v12→v13 rebuilds the `tasks` table (CREATE tasks_new, INSERT, DROP tasks, ALTER RENAME) but does NOT drop the `unified_timeline` VIEW beforehand. Since `unified_timeline` references `tasks`, on SQLite 3.25+ the `ALTER TABLE tasks_new RENAME TO tasks` step may fail because SQLite validates views during rename operations. The `DROP VIEW` at line 2035 comes AFTER the table rebuild, which is too late.

## Findings

- Location: `crates/mika-agent/src/db.rs` migrate_v12_to_v13 function
- The migration sequence is: CREATE tasks_new → INSERT INTO tasks_new → DROP tasks → ALTER RENAME tasks_new → DROP VIEW unified_timeline
- SQLite 3.25+ validates references in views during ALTER TABLE RENAME operations
- The `unified_timeline` view references the `tasks` table, causing the rename to fail
- The DROP VIEW statement exists but is placed after the rename, which is too late

## Proposed Solutions

Add `DROP VIEW IF EXISTS unified_timeline;` as the first statement in the migration batch, BEFORE the tasks table rebuild sequence.

## Acceptance Criteria

- [ ] Migration v12→v13 drops `unified_timeline` view before rebuilding the `tasks` table
- [ ] Migration succeeds on SQLite 3.25+
