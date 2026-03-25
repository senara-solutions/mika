# Brainstorm: Agent Runtime Observability — LLM Calls, Skills, Tool Calls

**Date:** 2026-03-25
**Status:** Draft
**Author:** AI-assisted brainstorm

## What We're Building

Three critical pieces of runtime data are either not stored or stored in truncated form, making it impossible to debug what the agent actually did during a turn. This brainstorm addresses all three gaps as a single coherent observability improvement:

1. **LLM calls are invisible** — No `llm_calls` table in SQLite. `info!` logs exist but default log level is `warn` so they never appear. OTel spans exist when `telemetry` feature is enabled but request/response bodies are not captured. After a turn completes, there's no record of which models were called, how many tokens were used, what the latency was, or what the stop reason was.

2. **Skills loading is silent on success** — Only failures produce warnings. There is no log or DB record of which skills loaded successfully, how many loaded, or which were skipped. The `skill_overrides` table tracks enable/disable toggles but not actual runtime loading. After startup, there's no way to verify skills are active without checking for the absence of error logs.

3. **Tool call output is aggressively truncated** — Tool inputs are truncated to 200 chars, outputs to 300 chars, total metadata capped at 4000 chars in `messages.metadata`. History injection truncates further to 60/80 chars. **Full tool input and output are never persisted anywhere** — they exist only in memory during the agent loop and are then discarded. This is the most painful gap: you cannot look at what a tool actually returned after the fact.

## Why This Approach

### The core insight

Mika already has the correlation infrastructure (`trace_id`, `session_id`, `unified_timeline` VIEW) from the orthogonal observability work. It already has a dashboard with timeline, traces, sessions views. It already has Langfuse-compatible OTel export. **The plumbing exists — the data doesn't.**

The fix is straightforward: store the data in SQLite tables, export it as OTel spans, and expose it through existing dashboard API patterns.

### Why SQLite + Langfuse (not one or the other)

- **SQLite** is the always-available local store. Works without external services, queryable from CLI and dashboard, supports full-text search. This is where the data lives.
- **Langfuse** (via OTel spans) provides the external trace viewer with timeline visualization, cost tracking, and the generation-level drill-down. The OTel export is already feature-gated and optional — same pattern applies to new spans.
- **Dashboard** is the primary UI. It already reads from SQLite via `/api/v1/*` endpoints. New tables get new endpoints, new dashboard views.

### Why all three gaps together

These aren't independent problems — they're all part of "what happened during this agent turn?" A single trace_id connects an LLM call to the tool calls it triggered to the skills that provided those tools. Solving them separately would mean three migrations, three PRs, three rounds of dashboard work. Solving them together means one migration, one coherent data model, one dashboard view that shows the full picture.

## Key Decisions

### 1. New `llm_calls` table in SQLite (configurable)

Store every LLM API call as a row:

| Column | Type | Notes |
|--------|------|-------|
| `id` | TEXT PK | UUID |
| `agent_id` | TEXT | FK to agents |
| `session_id` | TEXT | FK to sessions |
| `trace_id` | TEXT | Correlation |
| `provider` | TEXT | e.g. "anthropic", "openai" |
| `model` | TEXT | e.g. "claude-sonnet-4-20250514" |
| `input_tokens` | INTEGER | From API response |
| `output_tokens` | INTEGER | From API response |
| `cache_read_tokens` | INTEGER | Anthropic cache hits (nullable) |
| `cache_write_tokens` | INTEGER | Anthropic cache writes (nullable) |
| `latency_ms` | INTEGER | Wall-clock duration |
| `stop_reason` | TEXT | end_turn, tool_use, max_tokens, etc. |
| `status` | TEXT | success, error, timeout |
| `error_message` | TEXT | On failure (nullable) |
| `created_at` | TEXT | ISO 8601 |

**Gated by `store_llm_calls` config option** (default: `true`). When disabled, LLM calls are still logged and exported as OTel spans, but not written to SQLite. This lets production deployments opt out of the write overhead if they rely on Langfuse instead.

**What we DON'T store in the DB:** Request/response bodies. They're massive (system prompts alone are 10K+ tokens) and contain sensitive user data. Token counts + latency + stop reason are sufficient for post-hoc debugging. Full message content is already in the `messages` table. For development, see Decision 7 (log-level body dump).

### 2. New `tool_calls` table in SQLite (configurable)

Store every tool execution with full input and output:

| Column | Type | Notes |
|--------|------|-------|
| `id` | TEXT PK | UUID |
| `agent_id` | TEXT | FK to agents |
| `session_id` | TEXT | FK to sessions |
| `trace_id` | TEXT | Correlation |
| `llm_call_id` | TEXT | FK to llm_calls (which LLM call requested this tool) |
| `step` | INTEGER | Tool step number (0-9) |
| `tool_name` | TEXT | e.g. "search_memory", "mcp__github__search" |
| `tool_source` | TEXT | "builtin", "skill", "mcp" |
| `skill_name` | TEXT | If source=skill, which skill (nullable) |
| `input` | TEXT | **Full** JSON input (not truncated) |
| `output` | TEXT | **Full** output text (not truncated) |
| `success` | INTEGER | 0 or 1 |
| `non_zero_exit` | INTEGER | 0 or 1 (for exec handlers) |
| `latency_ms` | INTEGER | Wall-clock duration |
| `error_message` | TEXT | On failure (nullable) |
| `created_at` | TEXT | ISO 8601 |

**Gated by `store_tool_calls` config option** (default: `true`). When disabled, tool calls still produce OTel spans and truncated summaries in `messages.metadata` (existing behavior), but full I/O is not written to SQLite.

**Size concern:** Tool outputs can be large (exec handlers return up to 10K chars after truncation — but the *pre-truncation* output can be much larger). Two mitigations:
- Cap stored output at 50K chars (covers 99% of cases, prevents runaway storage)
- Add periodic cleanup: delete tool_calls older than 30 days (same pattern as session pruning)

### 3. Skills loading recorded at session start

When `SkillRegistry::from_dir()` completes, emit a structured summary:

- `info!` log with count and names of loaded skills
- Store in the `sessions` table metadata column: `{"loaded_skills": ["web-search", "self-knowledge", "file-reader"], "skipped_skills": ["broken-one"], "skill_count": 3}`
- One Langfuse span per session: `skill_loading` with `skill.count`, `skill.names`, `skill.skipped_count` attributes

**Always on — no config toggle.** This is tiny data (a JSON list of names in an existing column) with zero performance impact. There's no reason to turn it off.

### 4. OTel spans for all three

| Span name | Target | Attributes |
|-----------|--------|------------|
| `llm_call` | `mika::otel` | Already exists — add `latency_ms` attribute |
| `tool_call` | `mika::otel` | **New** — `tool.name`, `tool.source`, `tool.success`, `tool.latency_ms` |
| `skill_loading` | `mika::otel` | **New** — `skill.count`, `skill.names` |

These follow the existing pattern: `target: "mika::otel"` so they pass through the span filter. Feature-gated behind `telemetry`. Langfuse shows them as part of the trace.

### 5. Dashboard API endpoints and views

New endpoints (same auth pattern as existing `/api/v1/*`):

- `GET /api/v1/traces/{trace_id}/llm-calls` — LLM calls for a trace
- `GET /api/v1/traces/{trace_id}/tool-calls` — Tool calls for a trace
- `GET /api/v1/sessions/{id}/llm-calls` — LLM calls for a session
- `GET /api/v1/sessions/{id}/tool-calls` — Tool calls for a session
- `GET /api/v1/sessions/{id}/skills` — Loaded skills for a session (from metadata)
- `GET /api/v1/llm-calls?agent_id=&session_id=&model=&from=&to=&page=&per_page=` — Paginated LLM call list
- `GET /api/v1/tool-calls?agent_id=&session_id=&tool_name=&success=&from=&to=&page=&per_page=` — Paginated tool call list

Dashboard views:
- **Trace detail view** (existing) — add LLM calls panel and tool calls panel, expandable to show full I/O
- **Session detail view** (existing) — add "Skills" tab showing loaded/skipped skills, add "LLM Calls" and "Tool Calls" tabs
- **New "LLM Calls" page** — filterable list of all LLM calls across agents, with token usage stats and cost estimates
- **New "Tool Calls" page** — filterable list of all tool executions, expandable to show full input/output

### 6. Add to `unified_timeline` VIEW

Add `llm_calls` and `tool_calls` as UNION ALL legs in the `unified_timeline` VIEW:

```sql
-- llm_calls leg
SELECT trace_id, session_id, agent_id, 'llm_call' as event_type,
       provider || '/' || model as event_subtype,
       'tokens: ' || input_tokens || '→' || output_tokens || ' (' || latency_ms || 'ms)' as summary,
       created_at
FROM llm_calls

-- tool_calls leg
SELECT trace_id, session_id, agent_id, 'tool_call' as event_type,
       tool_name as event_subtype,
       CASE WHEN success THEN '✓' ELSE '✗' END || ' (' || latency_ms || 'ms)' as summary,
       created_at
FROM tool_calls
```

This means the existing timeline view automatically shows LLM calls and tool executions alongside messages, audit events, and tasks — no new query infrastructure needed.

### 7. Dev-mode LLM body logging

**New config option: `log_llm_bodies`** (default: `false`). When enabled, logs the full LLM request and response bodies to the log file at `debug` level:

- **Request:** system prompt, message history, tool definitions, model parameters
- **Response:** full assistant message content, tool_use blocks, stop reason

This is a **log-file-only, development-only** feature. It writes to the tracing subscriber (which goes to the log file and/or stdout depending on mode), NOT to SQLite. The bodies are massive and contain sensitive data — they should never be stored persistently or exported to Langfuse.

**Implementation:** In `ClaudeClient::send_message()` and `OpenAiCompatibleProvider::send_message()`, after serializing the request and after deserializing the response, emit `debug!(target: "mika::llm_debug", request_body = %json, "llm request")` and `debug!(target: "mika::llm_debug", response_body = %json, "llm response")`. The separate target (`mika::llm_debug`) means:
- Default log level (`warn`) hides them
- Setting `log_level = "debug"` shows everything (noisy)
- Setting `RUST_LOG=mika::llm_debug=debug` shows ONLY LLM bodies (precise)
- The `log_llm_bodies` config option adds `mika::llm_debug=debug` to the tracing filter automatically, so you don't have to fiddle with `RUST_LOG`

**Why not store in SQLite?** A single LLM request can be 50K-200K+ chars (system prompt + conversation history + tool definitions). Storing that per-call would blow up the database. For development debugging, the log file is ephemeral and appropriate. For production analysis of what was sent, reconstruct from `messages` table + skill prompts + config.

## Configuration Summary

Three new config options in `config.toml` (all with `MIKA_` env var prefix):

| Config key | Env var | Default | Effect |
|------------|---------|---------|--------|
| `store_llm_calls` | `MIKA_STORE_LLM_CALLS` | `true` | Write LLM call metadata to `llm_calls` SQLite table |
| `store_tool_calls` | `MIKA_STORE_TOOL_CALLS` | `true` | Write full tool I/O to `tool_calls` SQLite table |
| `log_llm_bodies` | `MIKA_LOG_LLM_BODIES` | `false` | Dump full LLM request/response JSON to log file (dev only) |

**On by default:** `store_llm_calls` and `store_tool_calls` default to `true` because the whole point is visibility. Users who don't want the storage overhead can opt out. This is observability — it should work out of the box.

**Off by default:** `log_llm_bodies` defaults to `false` because the output is enormous and may contain sensitive data. This is a developer tool, not a production feature.

## Data Flow

```
Agent turn starts
  │
  ├─ Skills matched → store skill list in session metadata (always)
  │                    emit skill_loading OTel span
  │
  ├─ LLM API call → if store_llm_calls: INSERT into llm_calls table
  │                  if log_llm_bodies: debug!() full request/response to log
  │                  emit llm_call OTel span (already exists, enhanced)
  │
  ├─ Tool execution → if store_tool_calls: INSERT into tool_calls table (full I/O)
  │                    emit tool_call OTel span (new)
  │                    still write truncated summary to messages.metadata (always, backward compat)
  │
  └─ Dashboard polls → new /api/v1/* endpoints serve the data
                        unified_timeline VIEW includes new event types
                        Langfuse shows full trace with LLM + tool spans
```

## Schema Migration (v14 → v15)

Single migration:
1. `CREATE TABLE llm_calls (...)` with indexes on `(trace_id)`, `(session_id)`, `(agent_id, created_at)`
2. `CREATE TABLE tool_calls (...)` with indexes on `(trace_id)`, `(session_id)`, `(llm_call_id)`, `(agent_id, created_at)`
3. Recreate `unified_timeline` VIEW with two new UNION ALL legs
4. No data migration needed — new tables start empty

## Scope Boundaries

### In scope
- `llm_calls` table + write path in both LLM providers
- `tool_calls` table + write path in agent loop `execute_tool()`
- Config options: `store_llm_calls`, `store_tool_calls`, `log_llm_bodies`
- Skills loading summary in session metadata + `info!` log
- OTel spans for tool calls and skill loading
- Dashboard API endpoints for new data
- Dashboard UI panels on trace and session detail views
- `unified_timeline` VIEW update
- 30-day retention cleanup for both new tables

### Out of scope
- LLM request/response body storage in SQLite (too large, privacy concerns — use `log_llm_bodies` for dev)
- Real-time streaming of tool execution to TUI (separate feature)
- Cost calculation engine (can be built on top of `llm_calls` later)
- Tool call replay/re-execution
- Breaking changes to existing `messages.metadata` format (backward compat maintained)

## Resolved Questions

- **Store request/response bodies in DB?** No — too large, sensitive data. Use `log_llm_bodies` config for dev-time debugging via log file.
- **Separate tables vs. extending messages.metadata?** Separate tables — metadata is already at its 4000-char limit and JSON-in-a-column is terrible for querying.
- **Where to store skill loading info?** Session metadata — skills are per-session, not per-turn.
- **Tool output size cap?** 50K chars — covers virtually all cases, prevents storage blowup.
- **Retention?** 30 days, same as existing session pruning pattern.
- **On or off by default?** SQLite storage on by default (the whole point is visibility). LLM body logging off by default (dev-only, massive output).
- **How to enable LLM body logging without RUST_LOG fiddling?** Dedicated `log_llm_bodies` config option that auto-adds the right tracing filter directive.

## Open Questions

None — all key decisions resolved during analysis.
