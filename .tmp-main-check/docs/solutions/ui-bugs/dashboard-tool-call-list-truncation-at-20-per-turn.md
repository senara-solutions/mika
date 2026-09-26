---
title: "Dashboard tool-call list truncates at 20 per turn, hides critical tool calls"
date: 2026-04-23
category: ui-bugs
module: dashboard/src/pages/SessionDetail.tsx
problem_type: ui_bug
component: tooling
severity: high
symptoms:
  - "Dashboard shows '20 TOOL CALLS' when the turn actually has 21+"
  - "run_claude_pilot dispatch call invisible on milestone-workflow turns"
  - "Operators conclude agent fabricated a dispatch when it actually succeeded"
root_cause: logic_error
resolution_type: code_fix
tags:
  - dashboard
  - observability
  - tool-calls
  - metadata
  - truncation
  - milestone-workflow
issue: 744
---

# Dashboard tool-call list truncates at 20 per turn, hides critical tool calls

## Problem

The dashboard's per-turn inline tool-call list silently dropped entries beyond what fit in a 4KB metadata JSON cap (`TOOL_METADATA_MAX = 4000`). Milestone-workflow orchestration turns routinely produce 21+ tool calls (6 `create_task` + 7 `update_task_status` + bookkeeping), and because `run_claude_pilot` executes last, it was precisely the most critical call that got hidden. The header count "20 TOOL CALLS" reinforced the lie because it matched the rendered count.

## Root Cause

The dashboard's `ToolCallsTable` component parsed tool calls from `messages.metadata` JSON — a column serialized by `tool_calls_metadata_json()` in `agent.rs` with a 4000-char cap. When the serialized array exceeded this budget, Phase 2 of the serializer dropped tail entries (keeping the first N that fit). Since `run_claude_pilot` typically executes last in a milestone-workflow turn, it was the first to be dropped.

Meanwhile, a dedicated `tool_calls` table (added in schema v15) stored all tool calls with a 50KB per-field cap and proper pagination. The `GET /api/v1/traces/:trace_id/tool-calls` endpoint and `useTraceToolCalls` React Query hook already existed — but the inline `ToolCallsTable` component never used them.

## Solution

Three changes:

1. **Expose `trace_id` on `MessageResponse`** (`crates/mika-agent/src/server/dashboard.rs`): Added `trace_id: Option<String>` to `MessageResponse` and its `From<SessionMessage>` impl. This was already stored in the DB — just not forwarded to the API response.

2. **Refactor `ToolCallsTable` to use API data** (`dashboard/src/pages/SessionDetail.tsx`, `TraceDetail.tsx`): The component now accepts an optional `traceId` prop, calls `useTraceToolCalls(traceId)` to fetch from the authoritative `tool_calls` table, and maps `ToolCallRow` to the local `ToolCall` interface. Falls back to `parseToolCalls(metadata)` when the API returns empty (backward compat for pre-v15 messages).

3. **Frontend type updates** (`dashboard/src/api/sessions.ts`, `timeline.ts`): Added `trace_id: string | null` to `Message` and `TraceMessage` interfaces.

The backend `TOOL_METADATA_MAX` cap was left unchanged — it still serves `format_tool_summary_block()` for LLM history context, where tail-drop is acceptable.

## Prevention

- **Prefer authoritative data sources over embedded summaries.** The `messages.metadata` JSON was a convenience copy with a size cap. The `tool_calls` table was the authoritative source with no practical limit. When both exist, always render from the authoritative source.
- **Budget math before caps.** When setting per-field truncation limits, calculate worst-case: `entries * (field_limits + overhead)` and verify it fits. The 4KB cap was designed for 10 entries but milestone-workflow turns routinely exceed that.
- **Silent truncation is a UI lie.** If display must be limited, show "N of M" — never silently drop entries with a header count that matches the rendered (truncated) count.

## Related

- `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md` — Original #115 fix that added the two-phase truncation strategy
- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — Original dashboard tool-call display UX
- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` — Architecture of the `tool_calls` table and API endpoints
