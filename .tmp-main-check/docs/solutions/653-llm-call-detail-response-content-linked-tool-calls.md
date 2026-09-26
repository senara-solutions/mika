---
module: dashboard, mika-agent
tags: [llm-calls, observability, schema-migration, dashboard-detail-page]
problem_type: feature-gap
category: architecture-patterns
---

# LLM Call Detail: Response Content, Linked Tool Calls, and Error State

## Problem

The LLM Calls detail page (`/dashboard/llm-calls/<id>`) displayed metadata only (provider, model, tokens, latency) but not the actual LLM response content or linked tool calls. Debugging required manually cross-referencing the `messages` table and `tool_calls` table via trace_id.

## Solution

### Schema Migration v30→v31

Added two nullable TEXT columns to `llm_calls`:
- `response_text` — serialized LLM response (text blocks joined with newlines, tool-call summaries as `[Tool Call: name(args)]`)
- `reasoning` — extended thinking text (Claude-only)

**Key design decisions:**
- **Store response, not prompt** — the prompt is the entire messages array (often 50KB+); the response is typically much smaller and is the primary debugging target.
- **50K char cap** using `truncate_chars()` — mirrors the tool_calls byte cap philosophy but uses char-boundary-safe truncation.
- **Strip internal tags at persistence boundary** — `strip_internal_tags()` applied before storage (not display-time), matching the `scrub_secrets()` pattern on tool_calls.
- **Per-column `column_exists` guards** in migration — handles crash-recovery where one column was added but not the other. Each ALTER TABLE is gated independently.

### Linked Tool Calls Endpoint

New `GET /api/v1/llm-calls/{id}/tool-calls` endpoint. Uses the existing `tool_calls.llm_call_id` FK and `idx_tool_calls_llm_call` index — no new indexes needed.

Pattern follows `GET /api/v1/traces/{id}/tool-calls` exactly: returns `Vec<ToolCallRow>`, empty array for unknown IDs (not 404).

### Frontend

- **Response panel** — pre block with CopyButton, max-h-96 overflow scroll
- **Reasoning panel** — collapsible (collapsed by default), lighter text color
- **Tool Calls section** — linked tool calls rendered as clickable rows with StatusBadge + latency
- **Error banner** — prominent red-tinted card when status='error', replaces the inline MetadataRow

### Performance

List queries (`row_to_llm_call`) hardcode `response_text: None, reasoning: None` — the columns are not selected in list queries. Only the detail query (`row_to_llm_call_detail`) reads columns 17-18. This keeps list page performance unchanged.

## Key Patterns

1. **Detail-vs-list deserialization split** — use separate `row_to_*` functions when large columns should only be fetched for detail views.
2. **Per-column migration guards** — when adding multiple columns in one migration, check each independently with `column_exists()` to handle partial-crash recovery.
3. **Persistence-boundary sanitization** — apply content transforms (strip_internal_tags, truncate_chars) before INSERT, not at query/display time.
4. **Additive schema migrations** — ALTER TABLE ADD COLUMN is safe in SQLite (no table rebuild, no data loss, fast execution).

## Files Modified

- `crates/mika-agent/src/db.rs` — migration, LlmCallRow struct, save/query functions
- `crates/mika-agent/src/async_db.rs` — async wrappers
- `crates/mika-agent/src/agent.rs` — response serialization at save site
- `crates/mika-agent/src/server/dashboard.rs` — new handler
- `crates/mika-agent/src/server/mod.rs` — route registration
- `crates/mika-agent/src/kg/{subject_extractor,entity_resolver}.rs` — pass None for new params
- `dashboard/src/api/llmCalls.ts` — interface + hook
- `dashboard/src/pages/LlmCallDetail.tsx` — full page enhancement
