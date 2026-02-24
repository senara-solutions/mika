---
status: pending
priority: p2
issue_id: "057"
tags: [code-review, database, operations, rust-v2]
dependencies: []
---

# memory_events Table Has No Retention or Cleanup Policy

## Problem Statement

The `memory_events` audit log table grows unboundedly. Every tool invocation that mutates memory (store_fact, update_core_memory) appends a row with before/after values. There is no pruning, no max-rows limit, and no TTL. Over months of use, this table will dominate SQLite file size.

**Location:** `crates/mika-agent/src/db.rs` — `log_memory_event()`, `memory_events` table

**Reported by:** security-sentinel, performance-oracle

## Findings

- `log_memory_event()` inserts unconditionally, no size checks
- `get_memory_events(session_id)` queries by session but no cleanup path exists
- before_value and after_value store full content (potentially large text blocks)
- No index on `created_at` for range-based cleanup queries

## Proposed Solutions

### Option A: Session-scoped retention with periodic cleanup (Recommended)
Keep events for the last N sessions (e.g., 100) or last 30 days. Add a `cleanup_old_events()` method called during startup.
- **Pros:** Simple, bounded growth, preserves recent audit trail
- **Cons:** Loses old history
- **Effort:** Small
- **Risk:** Low

### Option B: Row count limit with FIFO eviction
DELETE oldest rows when count exceeds threshold (e.g., 10,000 rows).
- **Pros:** Hard upper bound on table size
- **Cons:** Time-based is more intuitive for auditing
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] memory_events table has a bounded growth strategy
- [ ] Cleanup runs automatically (e.g., on Database::open)
- [ ] Test verifying cleanup removes old events while preserving recent ones

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | No urgent risk — Phase 1 usage is CLI-only with limited sessions |
