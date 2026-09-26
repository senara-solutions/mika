---
title: "Telemetry Schema — trace_id as First-Class Join Key"
date: 2026-03-25
status: implemented
brainstorm: docs/brainstorms/2026-03-25-telemetry-schema-trace-id-joins-brainstorm.md
pr: 274
---

# Plan: Telemetry Schema — trace_id as First-Class Join Key

## Context

The `/mika-turn-audit` command broke because it used `datetime()` time windows to correlate messages, LLM calls, tool calls, and audit events within a turn. The brainstorm concluded that `trace_id` is already the per-turn correlation key — it's populated on all telemetry tables and shared across all events in a single turn. The fix is not new FKs — it's making `trace_id` a first-class queryable dimension.

**Two gaps exist in the core product:**
1. Paginated API endpoints (`GET /api/v1/llm-calls`, `GET /api/v1/tool-calls`) don't support `trace_id` filtering
2. `llm_calls` table has no `step` column — can't distinguish which loop iteration produced a given LLM call within a turn

## Changes

### 1. Add `trace_id` to filter structs (`crates/mika-agent/src/db.rs`)

- Add `trace_id: Option<String>` to `LlmCallFilters`
- Add `trace_id: Option<String>` to `ToolCallFilters`
- Update `query_llm_calls()` and `query_tool_calls()` to append `AND trace_id = ?`

### 2. Add `step` column to `llm_calls` (schema v15→v16)

```sql
ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;
```

### 3. Update `save_llm_call()` to accept `step`

Add `step: u32` parameter. Update INSERT statement.

### 4. Pass `step` from agent loop (`crates/mika-agent/src/agent.rs`)

Pass `step as u32` to `save_llm_call()` at both success and error call sites.

### 5. Update async wrappers (`crates/mika-agent/src/async_db.rs`)

Pass `step` parameter through.

### 6. Wire `trace_id` through dashboard query params (`crates/mika-agent/src/server/dashboard.rs`)

Add `trace_id: Option<String>` to `LlmCallsQuery` and `ToolCallsQuery`.

### 7. Dashboard API types

Add `trace_id` filter and `step` field to TypeScript interfaces.

## Verification

- `cargo check` — compiles clean
- `cargo test` — 1690 tests pass, 0 failures
- `cargo clippy` — zero warnings
