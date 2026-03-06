---
status: pending
priority: p2
issue_id: "465"
tags: [code-review, correctness, task-engine, database]
dependencies: []
---

# 465 · `ensure_recurring_task` has no DB uniqueness constraint — duplicate recurring tasks possible

## Problem Statement

`ensure_recurring_task` reads all schedulable tasks, scans the `Vec` in
memory for a matching label, and only inserts if absent. This is a
check-then-act pattern with no DB-level guard. Concurrent calls (e.g.,
two startup paths, or a task tool creating a recurring task simultaneously
with startup) can both pass the check and insert two identical recurring
tasks. Both heartbeat and reflection would then fire simultaneously on every
tick. The `tasks` table has no `UNIQUE(agent_id, label)` constraint to
prevent this.

Additional issue: the check only scans `status IN ('pending','recurring_active')`.
If a recurring task was cancelled or expired, the next restart would create a
duplicate and both would accumulate in the DB.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/mod.rs:24–55`
- **DB schema:** `crates/mika-agent/src/db.rs` — `tasks` table has no uniqueness constraint on `(agent_id, label)`
- The entire check is O(N) in-memory scan against an unindexed full table fetch

## Proposed Solutions

### Option A — Add partial unique index + `INSERT OR IGNORE` (recommended)
In `migrate_v1`, add:
```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
    ON tasks(agent_id, label)
    WHERE trigger_type = 'recurring';
```
Change `create_task` (or add a `create_recurring_task`) to use `INSERT OR IGNORE`.
Remove the in-memory check from `ensure_recurring_task` entirely.

**Pros:** DB-level guarantee, race-free, O(1).
**Effort:** Small | **Risk:** Low

### Option B — Use `INSERT OR IGNORE` without index
Replace the read-check-insert pattern with a single `INSERT OR IGNORE INTO tasks ... ON CONFLICT(agent_id, label) DO NOTHING` after adding a unique constraint.

**Effort:** Small | **Risk:** Low

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs` (schema + new method), `crates/mika-agent/src/task_engine/mod.rs`

## Acceptance Criteria

- [ ] `UNIQUE` constraint or partial index on `(agent_id, label)` for recurring tasks
- [ ] `ensure_recurring_task` uses `INSERT OR IGNORE` instead of read-then-insert
- [ ] Test: call `ensure_recurring_task` twice with same label, assert exactly 1 DB row

## Work Log

- 2026-03-06: Identified by security (COR-6), architecture (ARCH-1, ARCH-10), and quality (QUAL-5) review agents
