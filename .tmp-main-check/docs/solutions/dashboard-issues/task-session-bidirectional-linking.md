---
title: "Task-session bidirectional linking via task_id column"
category: dashboard-issues
date: 2026-04-04
tags: [schema-migration, sessions, tasks, dashboard, observability, backward-compatibility]
modules: [mika-agent/db, mika-agent/server/dashboard, mika-agent/task_engine/dispatcher, dashboard/TaskDetail]
issue: "#436"
---

# Task-Session Bidirectional Linking

## Problem

The dashboard task detail page showed child tasks but couldn't link them to their sessions or traces. Sessions spawned by tasks (callbacks, reminders, heartbeats, delegates) had no reverse link. The CLI `--task-id` flag stored the correlation only in session metadata JSON — no indexed column for efficient queries.

## Root Cause

The `sessions` table had no `task_id` column. The only link was `tasks.created_by_session` (task → session), with no reverse direction. CLI sessions stored `task_id` in `metadata` JSON via `json_extract()`, but this was unindexed and not populated by dispatcher paths.

## Solution

### Schema v19: Add `task_id` to sessions

Simple `ALTER TABLE` migration — no full rebuild needed since no CHECK constraints change:

```sql
ALTER TABLE sessions ADD COLUMN task_id TEXT;
CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;
UPDATE sessions SET task_id = json_extract(metadata, '$.task_id')
  WHERE json_extract(metadata, '$.task_id') IS NOT NULL AND task_id IS NULL;
```

The backfill migrates existing CLI `--task-id` sessions from JSON metadata to the new column.

### Session creation function signatures

Added `task_id: Option<&str>` parameter to `create_session_with_metadata()` and `create_session_with_parent()`. All dispatcher paths (callback, reminder, heartbeat, reflection, skill_run) and `delegate_task` now thread `task.id` through session creation.

### Backward-compatible queries

Used `COALESCE(s.task_id, json_extract(s.metadata, '$.task_id'))` in all queries that filter by task_id. This handles both pre-v19 sessions (JSON-only) and post-v19 sessions (column). The COALESCE pattern appears in `list_sessions_paginated`, `count_sessions`, and `get_sessions_for_task_tree`.

### New API endpoint

`GET /api/v1/tasks/:id/sessions` — collects task IDs (root + direct children via `parent_task_id`), then finds sessions via COALESCE. Returns session ID, channel type, timestamps, message count, and joined task label.

### Frontend

- Child task rows in `TaskDetail.tsx` show clickable SESSION and TRACE badges (using existing `created_by_session` and `execution_trace_id` from `TaskResponse`)
- New "Sessions" card lists all sessions in the task tree with duration, message count, and relative timestamps

## Key Decisions

1. **`task_id` semantic:** The immediate trigger task ID (not the root task). Parent tasks are reachable via `tasks.parent_task_id` joins — keeps the data model normalized.
2. **ALTER TABLE, not rebuild:** Sessions table has no CHECK constraints to widen, so a simple column addition suffices.
3. **Dual storage in CLI path:** `mika ask --task-id` writes to both the metadata JSON (backward compat) and the new column. The JSON storage is transitional.
4. **Shallow tree traversal:** `get_sessions_for_task_tree` collects root + direct children only (not recursive). Sufficient for current task trees; deeper drilling uses the children endpoint.

## Prevention

- When adding nullable columns to existing tables, prefer `ALTER TABLE ADD COLUMN` over full table rebuilds (unless CHECK constraints need widening).
- Always backfill existing data in the migration when denormalizing from JSON metadata to a proper column.
- Use `COALESCE` for backward compatibility during the transition period when both old and new storage exist.
- Add partial indexes (`WHERE col IS NOT NULL`) for sparse nullable columns to avoid bloating the index.

## Related

- `docs/solutions/architecture-patterns/task-id-correlation-intermediate-calls.md` — original `--task-id` metadata storage
- `docs/solutions/dashboard-issues/add-restful-detail-pages-pattern.md` — 4-layer backend pattern
- `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md` — session/trace linking patterns
