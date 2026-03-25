---
title: "feat: agent runtime observability — LLM calls, tool calls, skills loading"
type: feat
status: active
date: 2026-03-25
origin: docs/brainstorms/2026-03-25-agent-runtime-observability-brainstorm.md
---

# Agent Runtime Observability — LLM Calls, Tool Calls, Skills Loading

## Overview

Three critical categories of runtime data are either not stored or aggressively truncated, making it impossible to debug agent behavior after the fact. This feature adds the data layer on top of Mika's existing correlation infrastructure (`trace_id`, `unified_timeline`, dashboard, Langfuse OTel): two new SQLite tables (`llm_calls`, `tool_calls`), session metadata for skills, config toggles, dashboard API endpoints, and dashboard UI.

(see brainstorm: `docs/brainstorms/2026-03-25-agent-runtime-observability-brainstorm.md`)

## Problem Statement

- **LLM calls invisible**: No DB record of model, tokens, latency, or stop reason. `info!` logs exist but default level is `warn`.
- **Skills loading silent**: Only failures produce warnings. No log or DB record of which skills loaded successfully.
- **Tool calls truncated**: Full I/O discarded; only 200/300-char summaries in `messages.metadata`. Cannot inspect what a tool returned after the fact.

## Proposed Solution

### Phase 1: Data Layer (Config + Schema + DB Functions)

**Config keys** (`crates/mika-common/src/config.rs`):

| Key | Env var | Default | Purpose |
|-----|---------|---------|---------|
| `store_llm_calls` | `MIKA_STORE_LLM_CALLS` | `true` | Gate LLM call writes to SQLite |
| `store_tool_calls` | `MIKA_STORE_TOOL_CALLS` | `true` | Gate tool call writes to SQLite |
| `log_llm_bodies` | `MIKA_LOG_LLM_BODIES` | `false` | Dev-only: dump full request/response to log |

**Schema migration v14 → v15** (`crates/mika-agent/src/db.rs`):

- `llm_calls` table: id, agent_id, session_id, trace_id, provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, latency_ms, stop_reason, status, error_message, created_at
- `tool_calls` table: id, agent_id, session_id, trace_id, llm_call_id (FK), step, tool_name, tool_source, skill_name, input (50K cap), output (50K cap), success, non_zero_exit, latency_ms, error_message, created_at
- Indexes on trace_id, session_id, llm_call_id, (agent_id, created_at)
- `unified_timeline` VIEW updated with `llm_call` and `tool_call` UNION ALL legs

**DB functions**: `save_llm_call()`, `save_tool_call()`, paginated query functions, by-trace queries, by-session queries, prune functions. Async wrappers in `AsyncDatabase`.

### Phase 2: Agent Loop Integration

**LLM call recording** (`crates/mika-agent/src/agent.rs`):
- `run_loop()` accepts `store_llm_calls: bool` and `store_tool_calls: bool`
- Timing via `Instant::now()` around `llm.send_message()`
- Records both success and error LLM calls
- Generates `llm_call_id` (UUID) passed to tool call recording for FK linkage
- All 3 call sites updated: `run_agent`, `run_silent_agent`, `run_team_agent`

**Tool call recording** (`crates/mika-agent/src/agent.rs`):
- `process_tool_calls()` accepts `store_tool_calls: bool` and `llm_call_id: Option<&str>`
- Timing around `execute_tool()`
- Detects tool source (builtin/skill/mcp) and skill name from dispatch chain
- Full input JSON and output text stored (50K cap)
- Existing `ToolCallSummary` / `messages.metadata` path unchanged (backward compat)

**Skills loading** (`crates/mika-agent/src/skills/mod.rs`):
- `info!` log with count, names, and skipped count after `scan_skills_dir()`
- Session metadata write: `{"loaded_skills": [...], "skill_count": N}`

**LLM body logging** (`crates/mika-common/src/claude.rs`, `crates/mika-common/src/llm/openai.rs`):
- `debug!(target: "mika::llm_debug")` for request and response bodies in both providers
- `logging::init()` / `init_pretty()` accept `log_llm_bodies: bool`, add `mika::llm_debug=debug` filter directive when true

### Phase 3: Dashboard API + UI

**7 new API endpoints** (`crates/mika-agent/src/server/dashboard.rs`, `mod.rs`):
- `GET /api/v1/llm-calls` — paginated with filters (agent_id, session_id, model, date range)
- `GET /api/v1/tool-calls` — paginated with filters (agent_id, session_id, tool_name, success, date range)
- `GET /api/v1/traces/{trace_id}/llm-calls`
- `GET /api/v1/traces/{trace_id}/tool-calls`
- `GET /api/v1/sessions/{id}/llm-calls`
- `GET /api/v1/sessions/{id}/tool-calls`
- `GET /api/v1/sessions/{id}/skills`

**Dashboard UI** (React/TypeScript):
- `dashboard/src/api/llmCalls.ts` — TanStack Query hooks
- `dashboard/src/api/toolCalls.ts` — TanStack Query hooks
- `dashboard/src/pages/LlmCalls.tsx` — filterable paginated table
- `dashboard/src/pages/ToolCalls.tsx` — filterable paginated table with expandable I/O rows
- `dashboard/src/pages/TraceDetail.tsx` — LLM Calls + Tool Calls panels added
- `dashboard/src/pages/SessionDetail.tsx` — 4-tab layout (Messages, LLM Calls, Tool Calls, Skills)
- `dashboard/src/App.tsx` — routes for `/llm-calls` and `/tool-calls`
- `dashboard/src/components/Sidebar.tsx` — nav links added

### Phase 4: Retention + Cleanup

- `prune_old_llm_calls(retention_secs)` and `prune_old_tool_calls(retention_secs)` in `db.rs`
- Called in `startup_recovery()` (`task_engine/engine.rs`) with 30-day retention

## Technical Considerations

- **Performance**: DB writes are fire-and-forget (`let _ = db.save_...`) so observability never blocks the agent loop. Writes go through the async DB channel (dedicated OS thread).
- **Storage**: Tool call output capped at 50K chars. 30-day retention prevents unbounded growth.
- **Backward compatibility**: Existing `messages.metadata` truncated summaries unchanged. New tables start empty.
- **Security**: LLM request/response bodies are NOT stored in SQLite (too large, sensitive). Dev-only `log_llm_bodies` writes to ephemeral log files only.

## System-Wide Impact

- **Interaction graph**: `run_loop()` → `llm.send_message()` → `save_llm_call()` → SQLite. `process_tool_calls()` → `execute_tool()` → `save_tool_call()` → SQLite. Dashboard polls via existing 5s refresh.
- **Error propagation**: DB write failures are silently dropped (observability should not crash the agent). Future: add `warn!` on failure.
- **State lifecycle**: New tables are append-only with 30-day pruning. No orphan risk — rows are self-contained (trace_id/session_id are stored, not FK-constrained).
- **API surface parity**: All new endpoints follow the existing `PaginatedResponse<T>` pattern with dashboard-or-internal token auth.

## Acceptance Criteria

- [x] `llm_calls` table stores model, provider, tokens, latency, stop_reason per LLM API call
- [x] `tool_calls` table stores full input/output (50K cap), source, skill name, timing per tool execution
- [x] Config toggles: `store_llm_calls` (default true), `store_tool_calls` (default true), `log_llm_bodies` (default false)
- [x] `unified_timeline` VIEW includes `llm_call` and `tool_call` event types
- [x] Skills loading logged at `info!` level with names + stored in session metadata
- [x] LLM body logging via `mika::llm_debug` tracing target in both Anthropic and OpenAI providers
- [x] 7 dashboard API endpoints serve the new data
- [x] Dashboard: LLM Calls page, Tool Calls page (expandable I/O), trace/session detail panels, skills tab
- [x] 30-day retention cleanup on startup
- [x] Schema v15 migration + clean-slate creation
- [x] All 1175 existing tests pass
- [x] Clippy clean
- [ ] UTF-8 safe truncation in `save_tool_call()` (SpecFlow finding — byte slicing can panic on multi-byte chars)
- [ ] Add `warn!` on `save_llm_call` / `save_tool_call` failures instead of silent drop

## Dependencies & Risks

- **No new crate dependencies** — uses existing `uuid`, `serde_json`, `tracing` crates
- **Schema migration**: Non-destructive (CREATE TABLE + CREATE INDEX only). No data migration needed.
- **Risk**: UTF-8 truncation panic on multi-byte tool output boundaries (identified by SpecFlow, fix pending)

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-25-agent-runtime-observability-brainstorm.md](docs/brainstorms/2026-03-25-agent-runtime-observability-brainstorm.md) — Key decisions: separate tables over extending messages.metadata, 50K output cap, config toggles default on, dev-only body logging via tracing target

### Internal References

- Orthogonal observability (trace_id correlation): `docs/brainstorms/2026-03-08-orthogonal-observability-brainstorm.md`
- Dashboard architecture: `docs/brainstorms/2026-03-08-observability-dashboard-brainstorm.md`
- Trace ID correlation pattern: `docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md`
- Langfuse span filtering: `docs/solutions/integration-issues/langfuse-non-llm-span-filtering.md`
- Dashboard tool calls UX: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md`

### Deferred Items

- OTel `tool_call` and `skill_loading` spans (brainstorm Decision 4) — deferred to follow-up
- `skipped_skills` in session metadata — deferred
- Recording investigate/compaction LLM calls (bypass `run_loop()`) — deferred
- Unit tests for new DB functions — deferred
