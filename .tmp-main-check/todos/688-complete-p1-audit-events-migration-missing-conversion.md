---
status: complete
priority: p1
issue_id: "688"
tags: [code-review, data-integrity, migration]
dependencies: []
---

## Problem Statement

The v11→v12 migration for `audit_events` and `audit_event_summaries` uses `INSERT INTO ..._new SELECT * FROM ...` without converting the `created_at` column from INTEGER to ISO 8601 TEXT. Every other table in the migration correctly uses `strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')`.

This silently stores bare integer strings (e.g., `"1741000000"`) in TEXT columns, breaking:
- `unified_timeline` VIEW ordering (integers sort before ISO dates in lexicographic comparison)
- `timestamp::parse()` / `format_ts()` calls on these values
- Timeline queries with ISO 8601 range filters
- `compact_old_audit_events` which uses `strftime('%Y', created_at)` on the TEXT column

## Findings

Found by: pattern-recognition-specialist, agent-native-reviewer, security-sentinel, code-simplicity-reviewer (all 4 agents flagged this)

**audit_events** — `db.rs` migrate_v11_to_v12:
```sql
INSERT INTO audit_events_new SELECT * FROM audit_events;
```

**audit_event_summaries** — same function:
```sql
INSERT INTO audit_event_summaries_new SELECT * FROM audit_event_summaries;
```

## Proposed Solutions

### Option A: Explicit column list with strftime conversion (Recommended)

Replace `SELECT *` with explicit column lists:

For `audit_events`:
```sql
INSERT INTO audit_events_new SELECT id, agent_id, session_id, tool_name,
    target_key, before_value, after_value, reasoning, trace_id, rewound_by_trace_id,
    strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
FROM audit_events;
```

For `audit_event_summaries`:
```sql
INSERT INTO audit_event_summaries_new SELECT id, agent_id, year, month,
    summary, event_count,
    strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
FROM audit_event_summaries;
```

- **Pros:** Consistent with all other tables in the migration, prevents data corruption
- **Cons:** None
- **Effort:** Small (two SQL statement changes)
- **Risk:** None

## Recommended Action

Option A — this is a straightforward fix.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs` (migrate_v11_to_v12 function)
- **Components:** Database migration, audit log, timeline VIEW

## Acceptance Criteria

- [ ] `audit_events` migration uses explicit column list with `strftime` conversion for `created_at`
- [ ] `audit_event_summaries` migration uses explicit column list with `strftime` conversion for `created_at`
- [ ] After migration, all `audit_events.created_at` values are valid ISO 8601 strings
- [ ] `query_timeline` returns correct results for audit events

## Work Log

- 2026-03-18: Identified during code review of timestamp migration changeset

## Resources

- All 4 review agents flagged this as critical
