---
status: ready
priority: p1
issue_id: "110"
tags: [code-review, performance, database]
dependencies: []
---

# Missing Index on memory_events.created_at

## Problem Statement

The `compact_old_memory_events` function queries `memory_events` with `WHERE created_at < ?1` and `ORDER BY created_at ASC`, plus a batch `DELETE FROM memory_events WHERE created_at < ?1`. There is no index on `created_at`, causing full table scans on both queries.

The `heartbeat_sends` table has `idx_heartbeat_sends_sent_at` for the same pattern, but `memory_events` was overlooked.

## Findings

- **Source:** performance-oracle review (CRITICAL-1)
- **Location:** `crates/mika-agent/src/db.rs` lines 1148-1153 (SELECT) and 1264-1266 (DELETE)
- **Evidence:** Only index on `memory_events` is `idx_memory_events_session ON memory_events(session_id)` (line 214). No index on `created_at`.
- **Current impact:** Negligible with small tables in Phase 1
- **Future impact:** At ~4,500 rows (90-day accumulation), full scan is tolerable but unnecessary. If compaction fails to run, table could grow to tens of thousands of rows.

## Proposed Solutions

### Option 1: Add index in schema migration v7
- **Pros**: Clean, simple, follows existing pattern (heartbeat_sends has same index)
- **Cons**: Requires schema version bump
- **Effort**: Small (one `CREATE INDEX` statement)
- **Risk**: Low

```sql
CREATE INDEX IF NOT EXISTS idx_memory_events_created_at ON memory_events(created_at);
```

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/db.rs` (migration function)
- **Related Components**: Compaction, scheduler
- **Database Changes**: Yes - new index, schema version 7

## Acceptance Criteria

- [ ] Index `idx_memory_events_created_at` exists after migration
- [ ] Schema version bumped to 7
- [ ] Existing tests pass
- [ ] `EXPLAIN QUERY PLAN` confirms index usage for compaction queries

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (performance-oracle)
**Actions:** Flagged as P1 performance issue
**Learnings:** Inconsistency with `heartbeat_sends` which already has the equivalent index

## Resources

- Commit under review: 38a843b
- Related: `idx_heartbeat_sends_sent_at` at db.rs line 254
