---
title: "Convert all SQLite timestamps from Unix i64 to ISO 8601 TEXT strings"
category: database-issues
date: 2026-03-18
tags:
  - sqlite
  - timestamps
  - schema-migration
  - iso8601
  - breaking-change
  - refactor
  - rust
  - react
  - task-engine
  - team-engine
  - dashboard-api
severity: medium
modules_affected:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/timestamp.rs
  - crates/mika-agent/src/task_engine
  - crates/mika-agent/src/teams
  - crates/mika-agent/src/server/dashboard.rs
  - crates/mika-agent/src/tools
  - crates/mika-cli/src/commands
  - dashboard/src
root_cause_type: design_debt
resolution_type: refactor
time_to_resolve: "~1 day"
---

# Convert All SQLite Timestamps from Unix i64 to ISO 8601 TEXT

## Problem

The original SQLite schema stored all timestamp columns as `INTEGER` (Unix epoch seconds). This had several practical drawbacks:

- Timestamps were opaque when inspecting the database directly
- Every display path independently re-implemented epoch-to-human conversion
- Time-range filtering required `unixepoch()` wrapping in every query
- The dashboard frontend had to perform `unix * 1000` multiplication before passing values to `new Date()`
- The `unified_timeline` VIEW could not sort or filter across heterogeneous sources without conversion functions

This reverses the direction taken in schema v9 (documented in [sqlite-datetime-format-mismatch.md](sqlite-datetime-format-mismatch.md)), which converted `reminders.fire_at` *from* ISO 8601 TEXT *to* Unix epoch INTEGER after discovering that SQLite's lexicographic comparison between `T`-separated ISO 8601 and space-separated `datetime('now')` silently broke due-reminder queries. The v12 approach avoids that trap by using a fixed-width format (`%Y-%m-%dT%H:%M:%SZ`) that sorts correctly as plain text and never mixes with `datetime('now')`.

## Root Cause

Design debt — the `INTEGER` convention was established in schema v3 and propagated through all subsequent tables. As the codebase grew (17 tables with timestamp columns, 12+ tool files, dashboard API, frontend), the scattered conversion logic became a maintenance burden.

## Solution

### Approach

Convert every timestamp column to `TEXT` using ISO 8601 UTC format (`%Y-%m-%dT%H:%M:%SZ`). This fixed-width format sorts correctly via lexicographic comparison, preserving the correctness of all `ORDER BY`, `<`, `>=`, and `BinaryHeap`-based queue comparisons without runtime arithmetic.

### Key Components

1. **New `timestamp` module** (`crates/mika-agent/src/timestamp.rs`) — centralized helpers:

```rust
pub const DB_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

pub fn now() -> String {
    Utc::now().format(DB_FORMAT).to_string()
}

pub fn format(dt: &DateTime<Utc>) -> String {
    dt.format(DB_FORMAT).to_string()
}

pub fn parse(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, DB_FORMAT) {
        return Ok(dt.and_utc());
    }
    // Fallback: try RFC 3339
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| anyhow!("invalid timestamp '{}': {}", s, e))
}

pub fn now_minus(d: Duration) -> String { format(&(Utc::now() - d)) }
pub fn now_plus(d: Duration) -> String  { format(&(Utc::now() + d)) }
```

2. **Schema migration v11->v12** — full table-rebuild pattern for 17 affected tables. SQLite does not support `ALTER COLUMN`, so each table is rebuilt:

```sql
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;

CREATE TABLE sessions_new (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ended_at TEXT,
    ...
);
INSERT INTO sessions_new SELECT id, agent_id,
    strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
    CASE WHEN ended_at IS NOT NULL
         THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch')
         ELSE NULL END,
    ...
FROM sessions;
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;
-- Recreate indexes...

COMMIT;
PRAGMA foreign_keys = ON;
```

3. **Struct field changes** — every `i64`/`Option<i64>` timestamp field changed to `String`/`Option<String>` across `Task`, `NewTask`, `Session`, `SessionMessage`, `AgentRow`, `TeamRunRow`, `QueuedTask`, and all dashboard response types.

4. **Backward-compatible checkpoint deserialization** — custom serde visitors in `teams/types.rs` handle both legacy integer and new string timestamps. `CHECKPOINT_VERSION` bumped from 1 to 2.

5. **Dashboard TypeScript** — filter fields changed from `number` to `string`; `formatTime` utilities updated to parse ISO 8601 instead of `epoch * 1000`.

### Review Fixes

Code review caught these issues before merge:

1. **`audit_events` and `audit_event_summaries` migration used `SELECT *`** (P1) — would have silently stored bare integer strings in TEXT columns, breaking timeline queries and `timestamp::parse()`. Fixed with explicit column lists and `strftime()` conversion.

2. **Dispatcher parse-failure fallback defaulted to `0`** (P2) — made the system treat a corrupt timestamp as "user just messaged," silently suppressing reflection and heartbeat. Fixed to default to `i64::MAX` (treat as stale) with a `warn!` log.

3. **Dead `SQL_NOW` constant** (P3) — defined but never used. Removed.

4. **Empty `default_timestamp()`** (P3) — returned `String::new()`, an invalid timestamp. Fixed to return `crate::timestamp::now()`.

5. **Stray `use chrono::Datelike` import** (P3) — misplaced outside test function. Moved to top of test module.

## Prevention Strategies

### Migration Safety Checklist

- **Never use `SELECT *` in migration copy steps.** Always enumerate columns explicitly and apply type conversions inline. This is the single most important rule — the P1 bug was exactly this.
- **Enumerate every table and view** that contains the column type being changed. Query `sqlite_master` or grep the schema DDL rather than relying on memory.
- **Post-migration verification:** Run `SELECT typeof(col), col FROM table LIMIT 10` to confirm the column type and value format after migration.
- **Treat view recreation as a migration step.** If any underlying table changes shape, the view must be dropped and recreated in the same migration.

### Code Review Patterns

- Flag any `SELECT *` in migration code — require explicit column lists
- Check that `unwrap_or(0)` or `unwrap_or_default()` on timestamp-derived values defaults in the correct direction (conservative, not permissive)
- Verify that any defined-but-unused constants are removed (or caught by `cargo clippy -D dead_code`)
- For struct field type changes, grep every construction site to confirm valid values

### Testing Recommendations

- Write migration integration tests that seed data at version N-1, run the migration, and assert `typeof()` and format of every converted column
- Add a `assert_iso8601(s: &str)` test utility for consistent format validation
- Test that `unified_timeline` returns correct `typeof` for rows from each source table
- Ensure `cargo clippy -- -D dead_code` runs in CI

### General Lessons

- **Enumerate, don't assume.** The most dangerous bugs in large refactors live in the tables and code paths you forgot to update.
- **Fallback values are load-bearing.** `unwrap_or(0)` is not neutral — it can silently corrupt scheduling logic.
- **`SELECT *` in migrations is permanently unsafe.** Any future column change risks silently propagating the wrong type.
- **Partial migrations are worse than no migration.** A mixed-type database is harder to reason about than either the old or new schema.

## Related Documentation

- [sqlite-datetime-format-mismatch.md](sqlite-datetime-format-mismatch.md) — The v9 migration that went the opposite direction (TEXT to INTEGER) due to format mismatch. Key counter-context for understanding why v12 uses fixed-width UTC format.
- [sql-column-mismatch-trace-detail-view.md](sql-column-mismatch-trace-detail-view.md) — Silent type coercion bug where INTEGER landed in an Option<String> column. Demonstrates the class of bugs from mixing integer and text timestamp assumptions.
- [consolidate-per-agent-team-dbs-into-single-container-db.md](consolidate-per-agent-team-dbs-into-single-container-db.md) — Established the `unixepoch()` convention that v12 converts away from.
- `docs/runtime-structure.md` — Full schema reference at version 12 with all TEXT timestamp columns.
- `docs/architecture.md` — Schema version changelog with v12 entry.
