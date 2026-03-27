---
title: "fix: Make create_work_item idempotent by deduplicating on reference_url"
type: fix
status: completed
date: 2026-03-28
issue: "#303"
---

# fix: Make create_work_item idempotent by deduplicating on reference_url

## Overview

When mika-dev encounters a tool failure (e.g., broken skill symlink) and recovers, it retries the entire workflow from scratch — including `create_work_item`. This produces duplicate work items: 13 `create_work_item` calls (3 failed, 10 succeeded) for a single task that should have created exactly 1 work item.

## Problem Statement

The `create_work_item` tool has **zero deduplication logic** at the code level. Every call generates a fresh UUID and inserts a new row. The only guard is a prompt instruction in `list_work_items`'s description: *"Check this before creating new work items to avoid duplicates"* — which is unreliable during recovery retries (established principle: [code guards over prompt instructions](../solutions/architecture-patterns/delegation-work-item-guard-enforcement.md)).

The existing five loop-prevention guards don't address this:
1. `is_task_context` blocks top-level creation — doesn't prevent duplicates
2. Depth cap of 3 — orthogonal
3. `is_callback_turn` blocks all creation — doesn't apply to recovery
4. Deferred `self_dev` source — prompt-only, not code-enforced
5. Max 5 per session — bypassed if recovery spawns new sessions

## Proposed Solution

Two-layer deduplication following the established pattern from [agent-creates-duplicates-after-compaction.md](../solutions/logic-errors/agent-creates-duplicates-after-compaction.md):

1. **DB-level partial unique index** on `(agent_id, reference_url)` for active manual tasks
2. **Tool-level catch** that returns the existing item's ID on constraint violation or label match

### Layer 1: DB Partial Unique Index (schema v17)

```sql
-- Migration: dedup existing duplicates first, then create index

-- Step 1: Cancel duplicate active work items (keep earliest per agent_id + reference_url)
UPDATE tasks SET status = 'cancelled', metadata = json_set(COALESCE(metadata, '{}'), '$.cancelled_reason', 'dedup_migration_v17')
WHERE id IN (
    SELECT t.id FROM tasks t
    INNER JOIN (
        SELECT agent_id, reference_url, MIN(created_at) as earliest
        FROM tasks
        WHERE trigger_type = 'manual'
          AND reference_url IS NOT NULL
          AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
        GROUP BY agent_id, reference_url
        HAVING COUNT(*) > 1
    ) dups ON t.agent_id = dups.agent_id
           AND t.reference_url = dups.reference_url
           AND t.created_at != dups.earliest
    WHERE t.trigger_type = 'manual'
      AND t.status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
);

-- Step 2: Create partial unique index (NULLs exempt — SQLite skips NULL in unique indexes)
CREATE UNIQUE INDEX idx_tasks_manual_active_ref_url
ON tasks(agent_id, reference_url)
WHERE trigger_type = 'manual'
  AND reference_url IS NOT NULL
  AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');
```

**NULL handling:** SQLite partial unique indexes skip rows where `reference_url IS NULL`. This is intentional — label-only dedup is handled at the tool level.

### Layer 2: Tool-Level Deduplication in `create_work_item.rs`

```
crates/mika-agent/src/tools/create_work_item.rs
```

**Two dedup paths:**

**Path A — reference_url provided:**
1. Attempt the INSERT as normal
2. On `SQLITE_CONSTRAINT_UNIQUE` (extended code 2067): query for the existing active item
3. Return success with `"Work item already exists: {id} — {label} (status: {status})"`

**Path B — no reference_url (label-only fallback):**
1. Before INSERT: query active manual tasks matching `agent_id` + `label COLLATE NOCASE`
2. If match found: return existing item (same format as Path A)
3. If no match: proceed with INSERT

**Response format for dedup hit:**
```
Work item already exists for this reference: {id}
Label: {existing_label}
Status: {existing_status}
Created: {created_at}

Reuse this work item ID for subsequent operations.
```

### Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Dedup key | `(agent_id, reference_url)` | URLs are stable across retries; labels may drift |
| Label matching | Case-insensitive exact match | LLMs vary casing; fuzzy matching is V2 |
| NULL reference_url | Tool-level label check only | DB can't enforce uniqueness on NULLs |
| Session counter on dedup | Don't increment | Dedup responses shouldn't consume quota |
| Guard ordering | Dedup runs AFTER existing guards | Blocked operations stay blocked (callback, depth) |
| Existing item response | Return as success | Agent needs the ID to proceed with delegation |
| Label mismatch on URL dedup | Return existing as-is, don't update | Avoids accidental label overwrites |
| `failed`/`delivered` in exclusion | Included | Agent should be able to retry after failure |

## Technical Considerations

### rusqlite Error Matching

The codebase should match on the specific UNIQUE constraint error. rusqlite exposes `Error::SqliteFailure(ffi::Error { extended_code, .. }, _)` where `extended_code == 2067` indicates `SQLITE_CONSTRAINT_UNIQUE`. Match on this, not the generic `ConstraintViolation`.

### Race Between Constraint Violation and SELECT

After catching the UNIQUE violation, the existing item could transition to a terminal state before the SELECT executes. Handle the empty-result case by retrying the INSERT once. If it fails again, return an error.

### Migration Safety

Existing databases from affected users will have duplicate active rows. The migration **must** deduplicate before creating the index, or `CREATE UNIQUE INDEX` will fail. Strategy: keep the earliest-created item per `(agent_id, reference_url)` group, cancel the rest with `cancelled_reason: "dedup_migration_v17"` in metadata.

### Guard Ordering

The dedup check runs **after** the existing five guards. This means:
- Callback turns still blocked (guard 3 fires first)
- Task context still requires parent_task_id (guard 1 fires first)
- Depth cap still enforced (guard 2 fires first)
- If all guards pass, then dedup check runs before INSERT

This prevents dedup from inadvertently relaxing security guards.

## System-Wide Impact

- **Interaction graph:** `create_work_item` → guards → dedup check → DB insert/return existing → audit log. The audit log should distinguish "created" from "dedup_reused".
- **Error propagation:** UNIQUE constraint violation is caught at tool level, converted to success response. No error propagates to agent loop.
- **State lifecycle risks:** Minimal — dedup migration cancels true duplicates with metadata breadcrumb. The partial index automatically excludes terminal items.
- **API surface parity:** Only `create_work_item` is affected. `list_work_items`, `check_work_item`, `update_work_item_status` are read/update tools unaffected by this change.

## Acceptance Criteria

- [x] `create_work_item` with duplicate `reference_url` returns existing item ID (not error)
- [x] `create_work_item` with duplicate label (no URL) returns existing item ID
- [x] `create_work_item` with same URL but all prior items in terminal states creates new item
- [x] Different `agent_id` with same URL creates separate items (index scoped by agent)
- [x] Migration deduplicates existing active duplicates before creating index
- [x] Migration on clean database creates index without error
- [x] Per-session counter (guard 5) NOT incremented on dedup hit
- [x] Existing guards (1-5) still fire before dedup check
- [x] Audit log records "dedup_reused" for dedup hits (distinct from "created")
- [x] Schema version bumped to 17

## MVP

### `crates/mika-agent/src/db.rs` — Migration v16→v17

Add dedup migration and partial unique index creation to the migration chain.

### `crates/mika-agent/src/db.rs` — New query: `find_active_work_item_by_ref_url`

```rust
/// Find an active manual work item by agent_id and reference_url
pub fn find_active_work_item_by_ref_url(
    &self,
    agent_id: &str,
    reference_url: &str,
) -> Result<Option<Task>> {
    // SELECT * FROM tasks WHERE agent_id = ? AND reference_url = ?
    //   AND trigger_type = 'manual'
    //   AND status NOT IN ('completed','cancelled','failed','delivered')
    // LIMIT 1
}
```

### `crates/mika-agent/src/db.rs` — New query: `find_active_work_item_by_label`

```rust
/// Find an active manual work item by agent_id and label (case-insensitive)
pub fn find_active_work_item_by_label(
    &self,
    agent_id: &str,
    label: &str,
) -> Result<Option<Task>> {
    // SELECT * FROM tasks WHERE agent_id = ? AND label = ? COLLATE NOCASE
    //   AND trigger_type = 'manual'
    //   AND status NOT IN ('completed','cancelled','failed','delivered')
    // LIMIT 1
}
```

### `crates/mika-agent/src/tools/create_work_item.rs` — Dedup logic

After existing guards pass:

1. If `reference_url` is `Some`: attempt INSERT; on `SQLITE_CONSTRAINT_UNIQUE`, call `find_active_work_item_by_ref_url`, return existing item
2. If `reference_url` is `None`: call `find_active_work_item_by_label`; if found, return existing item; else INSERT

On dedup hit: skip audit log "created" event, log "dedup_reused" instead, skip session counter increment.

### Tests

```
crates/mika-agent/src/tools/create_work_item.rs — mod tests
crates/mika-agent/src/db.rs — mod tests (migration, query tests)
```

- `test_create_work_item_dedup_by_reference_url` — same URL returns existing ID
- `test_create_work_item_dedup_by_label` — same label (no URL) returns existing ID
- `test_create_work_item_dedup_case_insensitive_label` — "Fix Bug" matches "fix bug"
- `test_create_work_item_allows_after_terminal` — completed item allows new creation
- `test_create_work_item_dedup_cross_agent` — different agents can have same URL
- `test_create_work_item_dedup_skips_session_counter` — counter unchanged on dedup
- `test_migration_v17_dedup_existing_duplicates` — migration handles pre-existing dupes
- `test_migration_v17_clean_database` — migration succeeds on clean DB

## Sources

- Issue: [#303](https://github.com/senara-solutions/mika/issues/303)
- Pattern: [agent-creates-duplicates-after-compaction.md](../solutions/logic-errors/agent-creates-duplicates-after-compaction.md) — three-layer defense (prompt + DB constraint + tool fallback)
- Pattern: [callback-task-loop-prevention.md](../solutions/architecture-patterns/callback-task-loop-prevention.md) — structural prevention over prompt-based
- Pattern: [delegation-work-item-guard-enforcement.md](../solutions/architecture-patterns/delegation-work-item-guard-enforcement.md) — code guards over prompt instructions
- Key files: `crates/mika-agent/src/tools/create_work_item.rs`, `crates/mika-agent/src/db.rs`
