---
status: complete
priority: p1
issue_id: "075"
tags: [code-review, performance, database]
dependencies: []
---

# Add indexes on conversations table for role and channel_type

## Problem Statement
The `conversations` table has no indexes beyond the implicit rowid. Every agent turn queries this table filtering on `role` and `channel_type`, causing full table scans. With 50+ messages (compaction threshold), this is 3-4 full scans per turn.

## Findings
- `count_messages()` — `WHERE role != 'summary'` (db.rs:857-860)
- `load_recent_messages()` — `WHERE role != 'summary' AND channel_type IN (...)` (db.rs:296-310)
- `load_messages_before_window()` — `WHERE role != 'summary' AND id < ?1` (db.rs:876-896)
- `load_conversation_summary()` — `WHERE role = 'summary'` (db.rs:839)
- `last_user_message_time()` — `WHERE role = 'user'` (db.rs:1006)
- All on the hot path (called every agent turn)
- Flagged by: Performance Oracle (P1)

## Proposed Solutions

### Option 1: Add composite indexes in v5 migration (Recommended)
```sql
CREATE INDEX IF NOT EXISTS idx_conversations_role ON conversations(role, id);
CREATE INDEX IF NOT EXISTS idx_conversations_channel_type ON conversations(channel_type, id);
```
**Pros:** Covers all query patterns, idempotent with IF NOT EXISTS
**Cons:** Slightly larger DB file
**Effort:** 10 minutes
**Risk:** Low

## Recommended Action
Option 1 — add to existing `migrate_v5()` function.

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs` — `migrate_v5()` function

## Acceptance Criteria
- [ ] Both indexes added to migrate_v5()
- [ ] Migration idempotent (IF NOT EXISTS)
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
**Actions:** Identified full table scans on hot-path queries
