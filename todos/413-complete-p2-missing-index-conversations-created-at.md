---
status: complete
priority: p2
issue_id: "413"
tags: [code-review, performance, database, reflection]
dependencies: []
---

# Missing Index on conversations.created_at Causes Full Table Scan

## Problem Statement

The new `get_conversations_since()` query filters on `created_at`:
```sql
WHERE created_at >= ?1 AND channel_type != 'reflection' ORDER BY id
```

There is no index on `conversations.created_at`. SQLite will perform a full table scan. For agents active for months with thousands of messages, this scans every row including large content TEXT columns.

## Findings

- **Performance oracle**: "For an agent active for 6 months at 100 msgs/day = ~18K rows scanned. At 1000 msgs/day = 180K rows at 6 months."
- Existing indexes: `idx_conversations_role` on `(role, id)`, `idx_conversations_channel_type` on `(channel_type, id)` — neither covers `created_at`
- Note: todo #075 (complete) previously addressed a similar missing index

## Proposed Solutions

### Option A: Add index in v10 migration (Recommended)
Add to the existing v10 migration:
```sql
CREATE INDEX IF NOT EXISTS idx_conversations_created_at ON conversations(created_at);
```
- **Pros**: Fixes the issue at source, efficient range scans
- **Cons**: Slightly larger DB, minor write overhead
- **Effort**: Small (1 line of SQL)
- **Risk**: Low

## Technical Details

- **Affected file**: `crates/mika-agent/src/db.rs` (migrate_v10 function)

## Acceptance Criteria

- [ ] Index on `conversations.created_at` exists after migration
- [ ] get_conversations_since() uses the index (verify with EXPLAIN QUERY PLAN)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-03 | Identified during code review | Previous todo #075 fixed similar issue |
