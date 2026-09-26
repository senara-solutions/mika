---
status: complete
priority: p3
issue_id: "560"
tags: [code-review, simplification]
dependencies: []
---

# Remove Redundant v2 Migration

## Problem Statement

The v1 base schema already includes `'delivered'` in the tasks CHECK constraint and `'tool_result'` in conversations CHECK. Since v1 already creates the correct schema, there is no v2. The `migrate_v2()` function and `CURRENT_SCHEMA_VERSION = 2` are unnecessary.

## Proposed Solutions

- Delete `migrate_v2()` function entirely
- Remove `if version < 2` branch from `run_migrations()`
- Keep `CURRENT_SCHEMA_VERSION = 1`
- The v1 schema is already correct for new databases; existing databases from before this branch don't exist yet (feature branch)

**Effort:** Small

## Acceptance Criteria

- [ ] `migrate_v2()` deleted
- [ ] `CURRENT_SCHEMA_VERSION` remains 1
- [ ] Fresh databases create correct schema with `delivered` and `tool_result` in one pass
- [ ] All tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | v1 already has the final schema |
| 2026-03-07 | Approved during triage | User clarified: no v2 needed, just delete the migration |
