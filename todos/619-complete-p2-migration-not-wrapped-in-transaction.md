---
status: complete
priority: p2
issue_id: 619
tags: [code-review, data-integrity, migration]
dependencies: []
---

# v7-to-v8 migration not wrapped in explicit transaction

## Problem Statement

The `migrate_v7_to_v8` method uses `execute_batch` for table rebuild (CREATE tasks_new, INSERT, DROP tasks, ALTER RENAME) without an explicit transaction. If the process crashes between DROP and RENAME, the database loses the tasks table permanently.

## Findings

- **Source**: Performance review agent, Architecture review agent
- **Location**: `crates/mika-agent/src/db.rs` — `migrate_v7_to_v8`

## Proposed Solutions

### Option A: Wrap in BEGIN IMMEDIATE...COMMIT (Recommended)
Prefix the batch with `BEGIN IMMEDIATE;` and suffix with `COMMIT;`.

- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Migration wrapped in explicit transaction
- [ ] Crash mid-migration rolls back cleanly
