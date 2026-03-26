# Brainstorm: Telemetry Schema — trace_id as the Join Key

**Date:** 2026-03-25
**Status:** Decided
**Repo:** mika

## Problem

The `/mika-turn-audit` and `/mika-task-audit` commands use time-window SQL queries (`BETWEEN datetime(..., '-2 minutes') AND datetime(..., '+1 minute')`) to correlate messages, LLM calls, tool calls, and audit events within a turn. This approach is fragile:

1. SQLite's `datetime()` doesn't parse ISO 8601 `T` separators correctly — queries silently return zero rows
2. Arbitrary time windows can miss data (long turns) or include unrelated data (fast consecutive turns)
3. It suggests the schema lacks direct relationships — but it doesn't

## Finding: trace_id Already Solves This

All four telemetry tables already carry `trace_id`:

| Table | trace_id populated | FK constraint |
|-------|-------------------|---------------|
| `messages` | 94% (100% for recent) | No |
| `llm_calls` | 100% | No |
| `tool_calls` | 100% | No |
| `audit_events` | 98% (100% for recent) | No |

A single `trace_id` is shared across all records within one turn (user message, LLM calls, tool calls, audit events, assistant message). Verified empirically: session `ef5eb956` shows user message, assistant message, 8 LLM calls, and 6 tool calls all sharing `trace_id = 26885afad47b43ba984039ff614dc3af`.

The `unified_timeline` VIEW already uses `trace_id` for cross-subsystem correlation.

## Decision: Use trace_id, No New FKs

**Rejected alternatives:**
- `user_message_id` FK on `llm_calls` — DRY violation (duplicates `trace_id` semantics), maintenance liability
- `turn_id` grouping key — already what `trace_id` is
- Both — over-engineered, two join paths to the same data

**The query pattern:**

```sql
-- Get last assistant message and its trace_id
SELECT trace_id, content FROM messages
WHERE agent_id = ? AND role = 'assistant'
ORDER BY created_at DESC LIMIT 1;

-- Get everything in that turn
SELECT * FROM messages WHERE trace_id = ?;
SELECT * FROM llm_calls WHERE trace_id = ?;
SELECT * FROM tool_calls WHERE trace_id = ?;
SELECT * FROM audit_events WHERE trace_id = ?;
```

No time windows. No `datetime()`. Just `trace_id`.

## Impact

### Audit commands (meta-repo)
- Rewrite `/mika-turn-audit` to use `trace_id` joins instead of time windows
- Rewrite `/mika-task-audit` similarly where applicable

### Optional schema enhancement
- Consider adding `step_index` to `llm_calls` to distinguish iteration order within a turn (call 0 = initial, call 1 = after first tool cycle, etc.)
- Consider wiring `llm_calls` and `tool_calls` into the `unified_timeline` VIEW

## Key Decisions

1. `trace_id` is the canonical turn-level correlation key — no new FKs
2. Audit commands must use `trace_id` for joins, never time windows
3. Existing schema is correct; the bug was in the queries, not the data model
