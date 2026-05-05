# Plan: Dashboard Team Runs Detail — Surface Telemetry & Fix Iteration Indicator

**Issue:** mika#652
**Branch:** `feat/652/dashboard-team-runs-missing-telemetry-no`
**Scope:** Bug-fix portion only (data-rendering, no design dependency)

## Problem

The Team Runs detail page (`/dashboard/team-runs/:runId`) renders the scaffold (goal, deliverable, status) but is missing actionable operational telemetry that already exists in the database. The `Iteration X/Y` indicator is potentially misleading when fewer iterations are captured. The "View Trace" link is a weak affordance — should be a chip with copy + link.

## Acceptance Criteria

1. **Duration** — Elapsed time displayed (computed from `started_at` / `ended_at`)
2. **Cost** — Aggregated LLM cost from `llm_calls` by trace_id (same formula as DevRunDetail)
3. **Agent roster** — List of participating agents with per-agent tool/LLM call counts
4. **Tool call summary** — Per-agent breakdown table (tool name, count, success rate)
5. **LLM call summary** — Per-agent breakdown table (model, input/output tokens, cache read, latency)
6. **Turn count** — Message count across agent sessions
7. **Iteration indicator accuracy** — Shows `Iteration X/Y` only when Y iterations exist in workspace data; shows `X of Y captured` when fewer exist
8. **TraceIdWidget** — Shared component using `<CopyButton />` from `@senara-solutions/ui` (per mika#665 AC3 constraint); replaces the plain "View Trace" link

## Architecture

### Data Flow (existing — no backend changes needed)

The backend already serves all necessary telemetry:
- `GET /api/v1/team-runs/:runId` — returns `TeamRunRow` with `trace_id`, `started_at`, `ended_at`, `iteration`, `max_iterations`
- `GET /api/v1/traces/:traceId/llm-calls` — returns `LlmCallRow[]` with tokens, latency, model, agent_id
- `GET /api/v1/traces/:traceId/tool-calls` — returns `ToolCallRow[]` with tool_name, success, agent_id
- `GET /api/v1/team-runs/:runId/workspace` — returns entries with iteration numbers

No new API endpoints required. All aggregation happens client-side (same pattern as TraceDetail page).

### Implementation Units

#### Unit 1: TraceIdWidget shared component

**File:** `dashboard/src/components/TraceIdWidget.tsx` (new)

A chip that:
- Displays truncated trace ID (first 12 chars + `...`)
- Uses `<CopyButton text={traceId} />` from `@senara-solutions/ui` (mika#665 constraint)
- Links to `/traces/:traceId` on click
- Optionally shows span count or LLM call count as a mini badge

Props:
```typescript
interface TraceIdWidgetProps {
  traceId: string
  llmCallCount?: number
  className?: string
}
```

#### Unit 2: Telemetry stats row

**File:** `dashboard/src/pages/TeamRunDetail.tsx` (modify)

Add a stats row below the header (same pattern as DevRunDetail's `StatBadge` components):
- Duration: compute from `started_at`/`ended_at`
- Cost: sum `llm_calls` cost (input_tokens * rate + output_tokens * rate — use simplified estimate or just show token totals)
- LLM calls count
- Tool calls count
- Agent count (distinct `agent_id` from tool/llm calls)

Fetch telemetry conditionally when `run.trace_id` exists using existing hooks:
- `useTraceLlmCalls(run.trace_id)`
- `useTraceToolCalls(run.trace_id)`

#### Unit 3: Agent roster & per-agent telemetry tables

**File:** `dashboard/src/pages/TeamRunDetail.tsx` (modify)

New collapsible section "Agent Telemetry" below the summary card:
- Group LLM calls by `agent_id` — show per-agent: model distribution, total tokens, total latency, cost estimate
- Group tool calls by `agent_id` — show per-agent: tool usage (top tools, success rate)
- Each agent links to `/agents/:agentId`

Use a compact table layout following TraceDetail patterns. Collapsible per-agent rows.

#### Unit 4: Fix iteration indicator

**File:** `dashboard/src/pages/TeamRunDetail.tsx` (modify)

Current: always shows `Iteration {run.iteration}/{run.max_iterations}` from the DB row.

Fix: Compare `run.iteration`/`run.max_iterations` against actual workspace entries. If `displayIterations.length < run.max_iterations`, show an explanatory note: `"Iteration {run.iteration}/{run.max_iterations} — {displayIterations.length} captured"`. If workspace shows all iterations, show normal `Iteration X/Y`.

#### Unit 5: Replace "View Trace" link with TraceIdWidget

**File:** `dashboard/src/pages/TeamRunDetail.tsx` (modify)

Replace lines 204-213 (the plain "View Trace" link) with `<TraceIdWidget traceId={run.trace_id} llmCallCount={llmCalls?.length} />`.

## File Changes Summary

| File | Change |
|------|--------|
| `dashboard/src/components/TraceIdWidget.tsx` | NEW — shared trace ID chip component |
| `dashboard/src/pages/TeamRunDetail.tsx` | MODIFY — add telemetry stats, agent roster, fix iteration, use TraceIdWidget |

## Out of Scope

- Iteration navigation/comparison UI (design-class, requires Stitch)
- Run Summary panel layout redesign (design-class)
- Cost calculation precision (we'll show token counts; precise cost needs model-specific pricing tables)
- Backend API changes (all data already available)

## Testing

- Build verification: `npm run build --prefix dashboard` must pass
- Visual: navigate to a completed team run with a trace_id and verify all telemetry sections render
- Edge case: team run without trace_id — telemetry section hidden gracefully
- Edge case: team run with 0 LLM/tool calls — empty states shown

## Dependencies

- `@senara-solutions/ui` CopyButton (already available)
- Existing `useTraceLlmCalls` and `useTraceToolCalls` hooks (already in `dashboard/src/api/`)
