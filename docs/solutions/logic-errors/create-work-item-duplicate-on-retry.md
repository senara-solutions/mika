---
title: "create_task produces duplicates during task recovery retries"
category: logic-errors
date: 2026-03-28
severity: medium
tags: [work-items, dedup, idempotency, migration, sqlite, partial-index]
related_issues: ["#303"]
---

# create_task produces duplicates during task recovery retries

## Problem

When mika-dev encounters a tool failure (e.g., broken skill symlink) and recovers, it retries the entire workflow from scratch — including `create_task`. Each retry creates a new task because the tool had zero code-level deduplication. Observed: 13 `create_task` calls (3 failed, 10 succeeded) for a single task.

The existing five loop-prevention guards (callback block, task context, depth cap, self_dev deferral, session cap) address different concerns and do not prevent duplicate creation with the same reference URL or label. The only dedup mechanism was a prompt instruction in `list_tasks`'s description, which is unreliable — especially during recovery where the agent re-enters the workflow from scratch.

## Root Cause

`create_task` performed a plain `INSERT INTO tasks` with a fresh UUID on every call. No uniqueness constraint existed on `(agent_id, reference_url)` or any natural key for manual tasks. The session cap (Guard 5, max 5) could be bypassed if recovery spawned new sessions.

## Solution

Two-layer deduplication following the established three-layer defense pattern from `agent-creates-duplicates-after-compaction.md`:

### Layer 1: DB Partial Unique Index (schema v17)

Migration `v16→v17` in `crates/mika-agent/src/db.rs`:

1. **Dedup existing duplicates** — cancels all but the earliest (by `rowid`) active task per `(agent_id, reference_url)` group, with `cancelled_reason: dedup_migration_v17` metadata breadcrumb.
2. **Create partial unique index** — `idx_tasks_manual_active_ref_url ON tasks(agent_id, reference_url) WHERE trigger_type = 'manual' AND reference_url IS NOT NULL AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')`.

NULLs are exempt (SQLite skips NULL values in unique indexes), so label-only dedup is tool-level only.

### Layer 2: Tool-Level Pre-Check

In `crates/mika-agent/src/tools/create_task.rs`, before INSERT:

- **Path A (reference_url provided):** Query `find_active_work_item_by_ref_url`. If found, return existing item ID with "already exists" message.
- **Path B (no reference_url):** Query `find_active_work_item_by_label` with `COLLATE NOCASE`. If found, return existing item.
- **Safety net:** UNIQUE constraint violation catch on INSERT as belt-and-suspenders for race conditions.

### Key Design Decisions

- **Dedup runs after security guards but before session cap** — returning an existing item is not a "new creation" and must not consume quota.
- **Audit log distinguishes `dedup_reused` from `created`** — enables monitoring dedup effectiveness.
- **`MIN(rowid)` as tiebreaker** in migration SQL — deterministic even when `created_at` timestamps collide (second-level precision).
- **Transaction boundary** on migration — `BEGIN IMMEDIATE` / `COMMIT` for atomicity, matching other migration patterns.

## Prevention

- **Established pattern:** Any tool that creates persistent records should follow the three-layer defense: (1) prompt instruction to check first, (2) DB UNIQUE constraint as hard guarantee, (3) tool-level constraint catching with graceful fallback. See `agent-creates-duplicates-after-compaction.md`.
- **Code guards over prompt instructions:** Per `delegation-work-item-guard-enforcement.md`, prompt-only enforcement is unreliable. If ignoring an instruction would cause real harm, enforce it in code.
- **New write tools** should have dedup from day one — adding it retroactively requires migration surgery.

## Related

- [agent-creates-duplicates-after-compaction.md](agent-creates-duplicates-after-compaction.md) — established the three-layer defense pattern
- [delegation-work-item-guard-enforcement.md](../architecture-patterns/delegation-work-item-guard-enforcement.md) — code guards over prompt instructions
- [callback-task-loop-prevention.md](../architecture-patterns/callback-task-loop-prevention.md) — structural prevention over prompt-based
- [work-item-tracking-manual-task-reuse.md](../architecture-patterns/work-item-tracking-manual-task-reuse.md) — five loop-prevention guards
