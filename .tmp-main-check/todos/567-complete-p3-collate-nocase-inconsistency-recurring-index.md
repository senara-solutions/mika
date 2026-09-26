---
status: complete
priority: p3
issue_id: 567
tags:
  - code-review
  - database
  - consistency
dependencies: []
---

# COLLATE NOCASE inconsistency between idx_tasks_unique_recurring and idx_tasks_unique_reminder

## Problem Statement

The existing `idx_tasks_unique_recurring` index uses `label` (default binary collation, case-sensitive), while the new `idx_tasks_unique_reminder` uses `label COLLATE NOCASE` (case-insensitive). For recurring reminders (`trigger_type='recurring'`, `action_type='send_message'`), both indexes apply — creating an inconsistency where the recurring index is case-sensitive but the reminder index is case-insensitive.

## Findings

- **File:** `crates/mika-agent/src/db.rs`, lines 613-630
- **Flagged by:** Architecture Strategist, Performance Oracle
- In practice, recurring tasks are system-created with stable labels, so case-sensitivity rarely matters
- The CLAUDE.md convention says "Case-insensitive COLLATE NOCASE on unique text columns" — the existing index violates this convention

## Proposed Solutions

### Option A: Add COLLATE NOCASE to idx_tasks_unique_recurring

Update the existing index to match the project convention. Requires a migration.

- **Pros:** Consistency, follows CLAUDE.md convention
- **Cons:** Requires index rebuild in migration
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Both indexes use COLLATE NOCASE on label column
- [ ] Migration handles existing databases

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Found during code review | Convention says NOCASE but existing index doesn't use it |
