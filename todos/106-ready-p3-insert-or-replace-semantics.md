---
status: ready
priority: p3
issue_id: "106"
tags: [code-review, data-integrity]
dependencies: []
---

# Replace INSERT OR REPLACE with ON CONFLICT DO UPDATE

## Problem Statement
`INSERT OR REPLACE` in SQLite deletes the existing row and inserts a new one, which resets the `rowid` and any columns not specified (like `created_at`). `ON CONFLICT DO UPDATE` preserves the row identity and unmentioned columns.

## Findings
- File: `crates/mika-agent/src/db.rs`
- INSERT OR REPLACE used for core_memory and potentially other upserts
- Semantically different from UPDATE: destroys and recreates the row
- Could reset auto-increment IDs and timestamps unexpectedly
- Flagged by: Data Integrity Guardian (Medium)

## Proposed Solutions

### Option 1: Use INSERT ... ON CONFLICT DO UPDATE (Recommended)
```sql
INSERT INTO core_memory (section, content) VALUES (?1, ?2)
ON CONFLICT(section) DO UPDATE SET content = excluded.content
```
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] INSERT OR REPLACE replaced with ON CONFLICT DO UPDATE where appropriate
- [ ] Row identity preserved on upserts
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Data Integrity Guardian identified destructive upsert semantics
