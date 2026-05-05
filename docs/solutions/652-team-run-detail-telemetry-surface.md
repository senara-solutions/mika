---
module: dashboard
tags: [team-runs, telemetry, trace, observability]
problem_type: missing-feature
category: dashboard
---

# Team Run Detail: Surfacing Telemetry from Existing Trace Endpoints

## Problem

The Team Runs detail page rendered the scaffold (goal, deliverable, status) but
lacked operational telemetry — no duration, no LLM/tool call counts, no per-agent
breakdown. The iteration indicator showed `X/Y` without clarifying when fewer
iterations were actually captured. The "View Trace" link was a weak affordance.

## Root Cause

The backend already served all telemetry data via `GET /traces/:traceId/llm-calls`
and `GET /traces/:traceId/tool-calls`. The frontend simply wasn't fetching or
rendering it. Team runs carry a `trace_id` that links to the same trace
infrastructure used by all other telemetry surfaces.

## Solution

### Approach: Client-side aggregation from existing endpoints

No backend changes needed. The frontend conditionally fetches trace-level data
when `run.trace_id` is available:

1. **Stats row** — Duration (computed from timestamps), LLM call count, tool call
   count, agent count, total tokens. Same `StatBadge` pattern as DevRunDetail.

2. **Agent Telemetry section** — Groups LLM/tool calls by `agent_id`, renders
   expandable per-agent cards with model distribution, token breakdown, latency
   stats, and top-5 tool usage.

3. **TraceIdWidget** — Shared component (chip with truncated ID, CopyButton from
   `@senara-solutions/ui`, link to trace detail). Uses CopyButton per mika#665
   AC3 forward constraint.

4. **Iteration indicator fix** — Compares workspace entries against
   `max_iterations`; shows "(N captured)" when fewer exist.

### Key pattern: Conditional query enablement

```typescript
const traceId = run?.trace_id ?? ''
const { data: llmCalls } = useTraceLlmCalls(traceId)
// Hook has `enabled: !!traceId` — empty string prevents fetch
```

This avoids waterfall requests (run must load first to get trace_id) while
keeping the hook interface clean.

### Key pattern: Client-side aggregation

`computeAgentTelemetry()` groups raw call arrays by agent_id into a typed
`AgentTelemetry` struct. This is the correct location for aggregation because:
- The trace endpoints already return all rows (no pagination needed for typical runs)
- Server-side aggregation would require a new endpoint for marginal benefit
- The pattern matches how TraceDetail already works

## Applicability

This pattern applies to any detail page that has a `trace_id` and needs to show
telemetry summaries. The TraceIdWidget component is reusable across:
- Dev Runs detail (child tasks with execution_trace_id)
- Session detail pages
- Any future surface that displays trace IDs
