---
title: "Runtime observability: LLM call and tool call recording in SQLite"
category: architecture-patterns
date: 2026-03-25
tags: [observability, sqlite, llm, tool-calls, dashboard, tracing, schema-migration]
modules: [mika-agent, mika-common, dashboard]
pr: 272
---

# Runtime Observability: LLM Call and Tool Call Recording

## Problem

Three categories of runtime data were invisible after agent turns completed:

1. **LLM calls**: No record of which models were called, token usage, latency, or stop reasons. `info!` logs existed but default log level (`warn`) hid them.
2. **Tool calls**: Full input/output discarded after use — only 200/300-char truncated summaries in `messages.metadata` (4KB cap). Impossible to inspect what a tool returned.
3. **Skills loading**: Completely silent on success. No way to verify which skills were active.

The correlation infrastructure existed (`trace_id`, `unified_timeline` VIEW, OTel export, dashboard) but the data didn't.

## Root Cause

The agent loop was designed for execution, not introspection. LLM responses were consumed and tool outputs were truncated for the next LLM turn — neither was persisted in a queryable form. The `messages.metadata` JSON column was the only storage, but its aggressive truncation (200-char input, 300-char output, 4KB total) made it useless for debugging.

## Solution

### Schema v15: Two new SQLite tables

**`llm_calls`**: id, agent_id, session_id, trace_id, provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, latency_ms, stop_reason, status, error_message, created_at. Indexed on trace_id, session_id, (agent_id, created_at).

**`tool_calls`**: id, agent_id, session_id, trace_id, llm_call_id (FK to llm_calls), step, tool_name, tool_source (builtin/skill/mcp), skill_name, input (full, 50KB cap), output (full, 50KB cap), success, non_zero_exit, latency_ms, error_message, created_at. Indexed on trace_id, session_id, llm_call_id, (agent_id, created_at).

Both tables added as UNION ALL legs in `unified_timeline` VIEW — automatically visible in existing timeline queries and dashboard.

### Config toggles

- `store_llm_calls` (default: true) — gate SQLite writes
- `store_tool_calls` (default: true) — gate SQLite writes
- `log_llm_bodies` (default: false) — dev-only full request/response dump via `mika::llm_debug` tracing target

### Agent loop integration

- `run_loop()` wraps `llm.send_message()` with `Instant::now()` timing, records success and error cases
- `process_tool_calls()` wraps `execute_tool()` with timing, detects tool source from dispatch chain
- Generated `llm_call_id` (UUID) links tool calls to the LLM call that triggered them
- DB write failures logged as `warn!` (never crash the agent loop)
- Skills metadata stored in `sessions.metadata` JSON column

### Dashboard

7 new API endpoints: `/llm-calls`, `/tool-calls`, `/traces/{id}/llm-calls`, `/traces/{id}/tool-calls`, `/sessions/{id}/llm-calls`, `/sessions/{id}/tool-calls`, `/sessions/{id}/skills`. Two new React pages (LlmCalls, ToolCalls) with expandable I/O rows. Trace and session detail views enhanced with LLM/tool/skills panels.

## Key Decisions

- **Separate tables over extending `messages.metadata`**: Metadata was already at its 4KB limit and JSON-in-a-column is terrible for querying.
- **No FK constraints on observability tables**: Soft references via trace_id/session_id. Allows independent 30-day pruning without cascade complications.
- **50KB output cap**: Covers 99% of tool outputs while preventing storage blowup. UTF-8 safe truncation via `is_char_boundary()`.
- **Fire-and-forget writes with `warn!`**: Observability should never break the agent loop. Errors are logged, not propagated.
- **Config on by default**: The whole point is visibility. Users who don't want the overhead can opt out.
- **`log_llm_bodies` via separate tracing target**: Uses `mika::llm_debug` (not `mika::otel`) to avoid sending sensitive content to Langfuse. The config option auto-adds the filter directive so users don't fiddle with `RUST_LOG`.

## Bugs Found and Fixed

1. **UTF-8 truncation panic**: Initial implementation used `&s[..N]` byte slicing which panics on multi-byte char boundaries. Fixed with `is_char_boundary()` walk-back.
2. **Silent error drops**: `let _ = db.save_...` silently discarded failures. Changed to `if let Err(e) = ... { warn!(...) }`.
3. **Stale `query_timeline` enum**: The tool's JSON schema listed `["message", "audit", "task"]` but the VIEW now has 6 event types. Agent couldn't filter by `llm_call`/`tool_call`. Fixed by updating the enum.
4. **`SkillSummary` type mismatch**: TypeScript type expected `{name, source, handler_type}` but API returned `{loaded_skills: string[]}`. Skills tab rendered blank columns. Fixed by simplifying the type.
5. **`TOOL_CALL_MAX_CHARS` naming**: Constant operated on bytes, not chars. Renamed to `TOOL_CALL_MAX_BYTES`.

## Prevention

- **Always use `is_char_boundary()` for string truncation** — never `&s[..N]` on user-facing strings. The `truncate_utf8_safe()` function in `db.rs` is the canonical helper.
- **When adding new event types to `unified_timeline` VIEW**, also update the `query_timeline` tool's `event_type` enum in `tools/query_timeline.rs`.
- **TypeScript types must match actual API response shapes** — test with real data before shipping dashboard features.
- **Observability DB writes should always be non-fatal** — use `if let Err(e) = ... { warn!() }`, never `?` propagation.

## Related

- Brainstorm: `docs/brainstorms/2026-03-25-agent-runtime-observability-brainstorm.md`
- Plan: `docs/plans/2026-03-25-002-feat-agent-runtime-observability-plan.md`
- Prior art: `docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md`
- UTF-8 precedent: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
- Langfuse span filtering: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
