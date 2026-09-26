---
title: "Memory-Aware Introspection Tool Pattern"
category: architecture-patterns
date: 2026-03-28
tags: [tools, introspection, memory, search, sqlite, like-escaping]
module: crates/mika-agent/src/tools/search_tool_history.rs
issue: 304
---

# Memory-Aware Introspection Tool Pattern

## Problem

The agent had no way to recall results of past tool calls across sessions. When the same information was needed again (e.g., "what did the web search return yesterday?"), the agent had to re-run the external call, wasting time and API credits. The `tool_calls` table already stored full input/output for observability, but this data was not queryable by the agent itself.

## Root Cause

The `tool_calls` table (schema v15) was designed for observability dashboards, not agent self-awareness. The dashboard API exposed filtering via `GET /api/v1/tool-calls`, but no equivalent builtin tool existed for the agent loop.

## Solution

Added `search_tool_history` as a builtin introspection tool registered in `default_tools()`, following the existing pattern established by `query_timeline`, `get_session_messages`, and `list_audit_events`.

### Key design decisions:

1. **Reuse existing infrastructure.** The tool wraps `query_tool_calls()` in `db.rs` — no new query methods needed. Added `keyword: Option<String>` to the existing `ToolCallFilters` struct with `LIKE`-based search.

2. **LIKE metacharacter escaping.** The keyword is user-supplied via the LLM. LIKE metacharacters (`%`, `_`) must be escaped to prevent unintended wildcard behavior. This follows the established pattern from P3-536 (team engine LIKE prefix vulnerability):

```rust
let escaped = keyword
    .replace('\\', "\\\\")
    .replace('%', "\\%")
    .replace('_', "\\_");
let like_pattern = format!("%{escaped}%");
// SQL: input LIKE ?N ESCAPE '\' OR output LIKE ?M ESCAPE '\'
```

3. **Output budget.** Each result's `input`/`output` truncated to 500 chars. Total output capped at 10KB (`MAX_OUTPUT_BYTES`). This follows the truncation-at-injection pattern from the callback-result-too-large solution.

4. **Agent scoping.** Non-orchestrator agents automatically scoped to own `agent_id` (same `is_orchestrator()` guard as `query_timeline`). Orchestrators see all agents' tool calls.

5. **Backward compatibility.** `keyword: None` added to all existing `ToolCallFilters` construction sites (dashboard handler).

### Context priority semantics

Also added a conflict-resolution paragraph to the system prompt Instructions section:

> current user message > core memory > active skill context > conversation summary > conversation history > search results

This establishes explicit priority when information from different sources conflicts, reducing hallucination in multi-source contexts.

## Prevention

- **New LIKE queries must escape metacharacters.** Any `LIKE '%{user_input}%'` pattern needs `%` and `_` escaping with an `ESCAPE` clause. Parameterized queries prevent SQL injection but not semantic wildcards.
- **New introspection tools follow the pattern:** register in `default_tools()`, use `is_orchestrator()` for scoping, validate inputs against `MAX_INPUT_LEN`, cap output size.
- **Extending `ToolCallFilters` (or similar filter structs)** requires updating all construction sites — search for struct name to find them.

## Related

- [Runtime Observability: LLM/Tool Call Recording](runtime-observability-llm-tool-call-recording.md)
- [Callback Result Too Large Causes Agent Timeout](../runtime-errors/callback-result-too-large-causes-agent-timeout.md)
- [ADR-007: Session Conversation Compaction Strategy](../../adr/007-session-conversation-compaction-strategy.md)
- [Memory Classification](../../memory-classification.md)
- Issue: #304
