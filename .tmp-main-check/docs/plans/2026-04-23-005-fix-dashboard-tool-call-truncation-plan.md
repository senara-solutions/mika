---
title: "fix: Remove 4KB metadata cap that silently truncates tool call display"
type: fix
status: active
date: 2026-04-23
---

# fix: Remove 4KB metadata cap that silently truncates tool call display

## Overview

The per-turn inline tool-call list in the dashboard silently drops tail entries when the serialized `messages.metadata` JSON exceeds the 4000-char `TOOL_METADATA_MAX` cap. This hides critical tool calls like `run_claude_pilot` that typically execute last in milestone-workflow orchestration turns, making successful dispatches look like fabrications.

## Problem Frame

The dashboard has two tool-call rendering paths: (1) inline per-message via `messages.metadata` JSON, and (2) a dedicated paginated `tool_calls` tab via the `tool_calls` DB table. Path 2 works correctly (no truncation, proper pagination). Path 1 is broken: `tool_calls_metadata_json()` in `agent.rs` caps serialized output at 4000 chars and drops tail entries when the budget is exceeded.

The cap was introduced as a safety measure (#115) when tool calls were only stored in metadata. Since then, schema v15 added a dedicated `tool_calls` table with a 50KB per-field cap. The metadata path is now redundant for dashboard rendering — the `tool_calls` table is the authoritative source.

The fix: make the inline `ToolCallsTable` component fetch from the `tool_calls` table API instead of parsing `messages.metadata`. This eliminates the 4KB cap entirely and uses the same authoritative data source as the paginated tab. The backend `TOOL_METADATA_MAX` cap remains unchanged — it still serves the history builder's `format_tool_summary_block()` for cross-turn introspection in the LLM context.

## Requirements Trace

- R1. Dashboard shows all tool calls for a turn, even with 21+ calls
- R2. The header count reflects the actual number of tool calls, not the truncated count
- R3. No silent truncation — if display is limited, a clear indicator shows "N of M"
- R4. Backend `TOOL_METADATA_MAX` cap is preserved for LLM history context (no regression)

## Scope Boundaries

- The `tool_calls_metadata_json()` backend function is NOT modified — it still serves `format_tool_summary_block()` for agent introspection
- The dedicated "Tool Calls" paginated tab is not changed — it already works correctly
- No schema migration needed

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/agent.rs:248` — `TOOL_METADATA_MAX = 4000` constant and `tool_calls_metadata_json()` function (lines 279-327) with tail-drop loop
- `dashboard/src/pages/SessionDetail.tsx:39-47` — `parseToolCalls()` parses `messages.metadata` JSON
- `dashboard/src/pages/SessionDetail.tsx:103-270` — `ToolCallsTable` renders inline per-message tool calls
- `dashboard/src/pages/TraceDetail.tsx:34-42, 170-270` — Duplicate `parseToolCalls()` and `ToolCallsTable` (same bug)
- `dashboard/src/api/toolCalls.ts` — Existing `useSessionToolCalls()` hook, `perPage = 50`
- `crates/mika-agent/src/server/dashboard.rs:963-986` — `GET /api/v1/sessions/:id/tool-calls` endpoint (paginated, works correctly)
- `crates/mika-agent/src/db.rs` — `get_tool_calls_by_trace()` or equivalent DB getter

### Institutional Learnings

- `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md` — Documents the original tail-drop fix (#115). The two-phase truncation was the right trade-off at the time but is now insufficient for 21+ call turns.
- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — Documents both rendering paths and confirms the `tool_calls` table is the authoritative source.

## Key Technical Decisions

- **Fetch from `tool_calls` table, not metadata**: The `tool_calls` table already stores full data with a 50KB cap. Rather than raising `TOOL_METADATA_MAX` (which would bloat every message row), we query the authoritative table by trace_id. This is the same data source the paginated tab uses.
- **New lightweight API query by trace_id**: The inline component needs tool calls for a specific LLM turn (identified by `trace_id` on the message). We need a way to fetch tool calls by trace_id. Check if `GET /api/v1/traces/:id/tool-calls` already exists or if we need a new endpoint. The existing session-level endpoint returns all tool calls for the session — we need per-trace filtering.
- **Keep metadata parsing as fallback**: For backward compatibility with messages stored before the `tool_calls` table existed, fall back to `parseToolCalls(metadata)` when the API returns empty results.

## Implementation Units

- [x] **Unit 1: Per-trace tool calls API hook (ALREADY EXISTS)**

The `useTraceToolCalls(traceId)` hook already exists in `dashboard/src/api/toolCalls.ts:55-61`, backed by `GET /api/v1/traces/:trace_id/tool-calls` endpoint in `dashboard.rs:915-928`. No work needed.

- [x] **Unit 2: Refactor `ToolCallsTable` to use API data**

**Goal:** Replace the metadata-parsing path in `ToolCallsTable` with API-fetched data from the `tool_calls` table, eliminating the 4KB truncation.

**Requirements:** R1, R2, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `dashboard/src/pages/SessionDetail.tsx`
- Modify: `dashboard/src/pages/TraceDetail.tsx`

**Approach:**
- Change `ToolCallsTable` props from `{ metadata }` to `{ traceId, metadata }` (metadata kept as fallback).
- Inside the component, call the new `useTraceToolCalls(traceId)` hook. If the API returns data, use it. If empty/error, fall back to `parseToolCalls(metadata)`.
- The header count now reflects the actual count from the authoritative source.
- Both `SessionDetail.tsx` and `TraceDetail.tsx` have duplicate `ToolCallsTable` components — update both. Consider extracting to a shared component if the duplication is straightforward to eliminate.

**Patterns to follow:**
- Existing TanStack React Query usage patterns in the dashboard
- The paginated tool-calls tab in `SessionDetail.tsx` for how to render API-sourced tool call data

**Test scenarios:**
- Happy path: Turn with 21 tool calls renders all 21 rows with correct header "21 tool calls"
- Happy path: Turn with 5 tool calls renders normally (no regression)
- Edge case: Turn with no trace_id (old data) falls back to metadata parsing
- Edge case: API error falls back to metadata parsing gracefully
- Integration: Header count matches the number of rendered rows

**Verification:**
- A turn with 21+ tool calls (like the reproduction case) shows all entries including `run_claude_pilot`
- The header reads "21 tool calls" (not "20 tool calls")
- Existing turns with fewer than 20 tool calls render unchanged

- [x] **Unit 3: Add regression test for 25+ tool call serialization**

**Goal:** Add a Rust unit test proving that the metadata tail-drop is observable and documenting the known limitation, plus a test confirming the `tool_calls` table path has no such limit.

**Requirements:** R1

**Dependencies:** None (can run in parallel with Units 1-2)

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (test module)

**Approach:**
- Add a test that creates 25 `ToolCallSummary` entries with realistic tool names (not pathologically long), calls `tool_calls_metadata_json()`, and asserts the returned JSON contains fewer than 25 entries — documenting the known limitation.
- Add a comment explaining that the dashboard now uses the `tool_calls` table instead of this metadata path, so the tail-drop is acceptable for the LLM history context use case.

**Patterns to follow:**
- `test_safety_net_drops_tail_on_overflow` in `agent.rs` — same test structure

**Test scenarios:**
- Happy path: 25 entries with realistic names → metadata contains fewer than 25 (documents the known cap)
- Happy path: All 25 entries retain `name` and `step` fields (structural integrity of kept entries)

**Verification:**
- Test passes, documenting the metadata cap behavior as known and acceptable

## System-Wide Impact

- **Interaction graph:** The `ToolCallsTable` component gains an API dependency (React Query fetch) where it previously only parsed props. This adds a network request per assistant message with tool calls. The request is lightweight (tool call rows are small) and cached by React Query.
- **Error propagation:** API fetch failure falls back to metadata parsing — no regression path.
- **API surface parity:** Both `SessionDetail` and `TraceDetail` pages have identical `ToolCallsTable` components that must be updated in lockstep.
- **Unchanged invariants:** `tool_calls_metadata_json()` behavior is unchanged. `format_tool_summary_block()` continues to work from metadata. The paginated "Tool Calls" tab is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Additional API requests per message | React Query caching. Tool call data is small and static (immutable once written). |
| Missing trace_id on old messages | Fallback to metadata parsing preserves backward compatibility |

## Sources & References

- Related issue: #744
- Related solution: `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md`
- Related solution: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md`
