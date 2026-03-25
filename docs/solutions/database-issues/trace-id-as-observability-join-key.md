---
title: "trace_id as the canonical observability join key"
category: database-issues
date: 2026-03-25
tags: [observability, telemetry, trace_id, llm_calls, tool_calls, schema, sqlite]
related_prs: [272, 274]
---

# trace_id as the Canonical Observability Join Key

## Problem

The `/mika-turn-audit` command used `datetime()` time windows to correlate messages, LLM calls, tool calls, and audit events within a single agent turn. This failed silently because SQLite's `datetime()` function doesn't parse ISO 8601 timestamps with the `T` separator (e.g., `2026-03-25T16:40:16`). Queries returned zero rows despite data existing.

Investigation revealed the underlying assumption was wrong: there appeared to be no direct relationship between the telemetry tables (no FK constraints on `llm_calls` or `tool_calls`). The time window was a workaround for missing joins.

## Root Cause

The telemetry tables already had the right join key — `trace_id` — but it wasn't being used:

- `trace_id` (32-char hex) is set once per turn and shared across user messages, LLM calls, tool calls, audit events, and assistant messages
- Verified empirically: 100% of `llm_calls` and `tool_calls` have `trace_id` populated
- The `unified_timeline` VIEW already joins on `trace_id`
- No new FKs are needed — `trace_id` is the canonical per-turn correlation key

The gap was in the API layer: the paginated endpoints (`GET /api/v1/llm-calls`, `GET /api/v1/tool-calls`) didn't support `trace_id` as a filter parameter, and `llm_calls` lacked a `step` column to distinguish iterations within a turn.

## Solution

Schema migration v15→v16:
```sql
ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;
```

Rust changes:
- Added `trace_id: Option<String>` to `LlmCallFilters` and `ToolCallFilters`
- Updated `query_llm_calls()` and `query_tool_calls()` to build `WHERE trace_id = ?` clauses
- Added `step: u32` parameter to `save_llm_call()` (sync + async wrappers)
- Agent loop passes `step as u32` from the existing `for step in 0..max_steps` counter
- Updated `LlmCallsQuery` and `ToolCallsQuery` dashboard structs

Dashboard API types updated with `trace_id` filter and `step` field.

## Query Pattern

The correct way to get all telemetry for a turn:

```sql
-- 1. Get the last assistant message and its trace_id
SELECT trace_id FROM messages
WHERE agent_id = ? AND role = 'assistant'
ORDER BY created_at DESC LIMIT 1;

-- 2. Get everything in that turn by trace_id
SELECT * FROM messages WHERE trace_id = ?;
SELECT * FROM llm_calls WHERE trace_id = ? ORDER BY step;
SELECT * FROM tool_calls WHERE trace_id = ? ORDER BY step;
SELECT * FROM audit_events WHERE trace_id = ?;
```

No time windows. No `datetime()`. Just `trace_id`.

## Prevention

- Never use `datetime()` with ISO 8601 `T`-separated timestamps in SQLite — use string comparison instead (lexicographic ordering works correctly for fixed-width ISO 8601)
- When adding new telemetry tables, always include `trace_id TEXT` as a correlation column
- Use `trace_id` joins, not temporal proximity, for within-turn correlation
