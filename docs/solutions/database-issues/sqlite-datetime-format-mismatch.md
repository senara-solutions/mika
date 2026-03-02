---
title: "SQLite datetime format mismatch prevents reminder detection"
category: database-issues
component: crates/mika-agent/src/db.rs
tags: [sqlite, datetime, reminder, scheduler, migration, schema, unix-timestamp]
date_identified: 2026-03-02
date_resolved: 2026-03-02
severity: high
affected_modes: [cli-chat, server]
---

# SQLite Datetime Format Mismatch Prevents Reminder Detection

## Problem

Reminders created via the `create_reminder` tool were stored in SQLite but never detected as past-due by the scheduler's queries. The background poller (added in the [companion fix](../runtime-errors/reminders-never-fire-at-scheduled-time.md)) polled every 60 seconds but always found zero due reminders. The `mika reminders` CLI also showed overdue reminders as still "pending."

### Symptoms

- User creates a reminder: "remind me in 2 minutes to test"
- Reminder stored with correct `fire_at` timestamp
- Poller runs every 60s, finds nothing
- `SELECT fire_at <= datetime('now') FROM reminders WHERE status='pending'` returns `0` for all rows
- Confirmed against live database: two overdue reminders both show `is_past_due = 0`

## Root Cause

The `fire_at` column stored ISO 8601 text like `"2026-03-02T15:01:11Z"` (with **`T` separator**), but the SQL query compared it against `datetime('now')` which returns `"2026-03-02 15:05:49"` (with **space separator**).

In SQLite's lexicographic string comparison, `'T'` (ASCII 84) > `' '` (ASCII 32), so:

```
"2026-03-02T15:01:11Z" <= "2026-03-02 15:05:49"
         ^                          ^
         T (84)          >     space (32)
         → always FALSE
```

Every `fire_at <= datetime('now')` comparison was false, regardless of the actual times involved. No reminder was ever detected as past-due.

## Solution

Schema migration v9 converts `fire_at` from TEXT to INTEGER (Unix timestamps). Integer comparison is unambiguous — no format parsing, no separator issues.

### Migration SQL

```sql
BEGIN;

-- Add new integer column
ALTER TABLE reminders ADD COLUMN fire_at_unix INTEGER;

-- Backfill: unixepoch() handles both 'T' and space separators
UPDATE reminders SET fire_at_unix = unixepoch(fire_at);

-- Guard: mark any reminders with unparseable fire_at as failed
UPDATE reminders SET status = 'failed'
    WHERE fire_at_unix IS NULL AND status = 'pending';
-- Safe default for non-pending rows with NULL
UPDATE reminders SET fire_at_unix = 0 WHERE fire_at_unix IS NULL;

-- Must drop index before dropping column (SQLite constraint)
DROP INDEX IF EXISTS idx_reminders_status_fire_at;

-- Drop old text column (requires SQLite 3.35+)
ALTER TABLE reminders DROP COLUMN fire_at;

-- Rename new column
ALTER TABLE reminders RENAME COLUMN fire_at_unix TO fire_at;

-- Recreate index on integer column
CREATE INDEX idx_reminders_status_fire_at ON reminders(status, fire_at);

INSERT INTO schema_version (version) VALUES (9);
COMMIT;
```

### Key Design Decisions

1. **Unix timestamps over formatted text:** Integer comparison (`fire_at <= unixepoch('now')`) is O(1) per row, unambiguous, and more compact in the B-tree index.

2. **NULL guard during migration:** `unixepoch()` returns NULL for unparseable input. The migration marks affected pending reminders as `'failed'` and sets remaining NULLs to `0`, preventing "ghost" rows that are invisible to all queries.

3. **Tool API unchanged:** `create_reminder` still accepts ISO 8601 from the LLM and converts internally via `parsed.timestamp()`. The schema change is invisible to the agent.

4. **Centralized display formatting:** `Reminder::display_fire_at()` converts timestamps back to human-readable `"YYYY-MM-DD HH:MM:SS UTC"` in a single method, used by all 5 display sites.

## Implementation Details

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Migration v9, `Reminder.fire_at: i64`, `add_reminder(i64, &str)`, queries use `unixepoch('now')`, `display_fire_at()` helper |
| `crates/mika-agent/src/async_db.rs` | Updated `add_reminder` signature |
| `crates/mika-agent/src/tools/create_reminder.rs` | Store `parsed.timestamp()`, display human-readable in success message |
| `crates/mika-agent/src/tools/list_reminders.rs` | Use `r.display_fire_at()` |
| `crates/mika-agent/src/tools/search_memory.rs` | Use `r.display_fire_at()` |
| `crates/mika-agent/src/tools/cancel_reminder.rs` | Updated test timestamps |
| `crates/mika-agent/src/scheduler.rs` | Updated test timestamps, fixed stale doc comment |
| `crates/mika-cli/src/commands/reminders.rs` | Use `r.display_fire_at()` |
| `crates/mika-cli/src/tui/commands/handlers.rs` | Use `r.display_fire_at()` |

### Query Changes

```rust
// Before (broken — lexicographic comparison)
"WHERE status = 'pending' AND fire_at <= datetime('now')"

// After (correct — integer comparison)
"WHERE status = 'pending' AND fire_at <= unixepoch('now')"
```

### Display Helper

```rust
impl Reminder {
    pub fn display_fire_at(&self) -> String {
        DateTime::from_timestamp(self.fire_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| self.fire_at.to_string())
    }
}
```

## Verification

1. `cargo test` — 745 tests pass
2. `cargo clippy` — no warnings
3. Live DB check: `SELECT fire_at, typeof(fire_at) FROM reminders` shows integers
4. `SELECT fire_at <= unixepoch('now') FROM reminders WHERE status='pending'` returns `1`

## Prevention Strategies

- **Use INTEGER timestamps for comparison columns.** Text datetimes are fine for display-only columns (`created_at`, `delivered_at`), but any column used in `WHERE` comparisons should be INTEGER to avoid format ambiguity.
- **Test actual SQL queries, not just Rust logic.** The Rust code correctly parsed and stored ISO 8601, but the SQL comparison silently failed. Integration tests should run real `SELECT` queries.
- **Verify format compatibility when using SQLite datetime functions.** `datetime('now')` returns space-separated format; ISO 8601 uses `T`. These are not interchangeable for comparison.
- **Watch for silent logic failures in time-sensitive features.** Reminders not firing, expirations not triggering, or scheduled tasks misfiring — check the actual SQL comparison first.

## Lessons Learned

- **A single character difference can break an entire feature.** `' '` vs `'T'` silently caused all reminder comparisons to return false, with no runtime errors.
- **Schema design is the first line of defense.** Choosing INTEGER timestamps upfront prevents format debates and makes comparison logic trivially correct.
- **`datetime('now')` and ISO 8601 are not the same format.** SQLite's datetime functions output space-separated timestamps, not `T`-separated ISO 8601.

## Related

- [Reminders never fire at scheduled time](../runtime-errors/reminders-never-fire-at-scheduled-time.md) — companion fix that added the background poller
- [Architecture: Database Schema](../../architecture.md#appendix-database-schema) — schema version 9 reference
