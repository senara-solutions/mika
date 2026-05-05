---
module: dashboard
tags: [chart, time-series, recharts, cost, pricing, llm-calls, aggregation, sqlite]
problem_type: feature
category: dashboard
date: 2026-05-05
issue: 660
---

# Dashboard Cost Trend Chart — Time-Series Visualization

## Problem

The Mika observability dashboard was entirely table-based with zero charts. For an operator monitoring LLM API spend, cost-over-time trends are the highest-signal visualization — they answer "am I spending more or less than usual?" at a glance, which tables cannot.

## Solution

Added a `<CostTrendChart>` component to the LLM Calls page with:

### Backend (Rust)

1. **Pricing module** (`crates/mika-agent/src/pricing.rs`): Server-side cost estimation from token counts using a per-provider/model pricing table. Covers Anthropic, OpenAI, DeepSeek, Google, Groq, Mistral, and Ollama (zero-cost). 4-component cost formula accounting for cache read tokens (10% input rate for Anthropic) and cache write tokens (125% input rate).

2. **Aggregation endpoint** (`GET /api/v1/llm-calls/cost-trend`): SQL query groups `llm_calls` by time bucket + agent_id + provider + model, then applies pricing per row in Rust. Returns `CostTrendResponse` with pre-aggregated buckets, bucket size, and estimated-pricing metadata.

3. **Auto-bucketing**: Server determines granularity from the from/to span: hourly for <3 days, daily for >=3 days. Defaults to last 24h when no `from` param.

### Frontend (React + TypeScript)

4. **CostTrendChart component** (`dashboard/src/components/CostTrendChart.tsx`): recharts `<AreaChart>` with two variants:
   - **Total**: Single accent-colored area with gradient fill
   - **By Agent**: Stacked areas with deterministic agent color palette

5. **Integration**: Chart sits above the table on the LLM Calls page, sharing the same `from`/`to`/`agent_id` URL params as the table filter.

## Key Patterns

### SQLite Time-Bucket Aggregation

Use `substr()` instead of `strftime()` for bucketing ISO 8601 timestamps stored as TEXT:

```sql
-- Hourly: substr(created_at, 1, 13) || ':00:00Z'  → "2026-05-05T10:00:00Z"
-- Daily:  substr(created_at, 1, 10) || 'T00:00:00Z' → "2026-05-05T00:00:00Z"
```

This avoids the `T` vs space format mismatch with SQLite's `strftime()` function (see `docs/solutions/database-issues/sqlite-datetime-format-mismatch.md`).

### Server-Side Cost Computation

Cost is computed server-side, not client-side, because:
- Single source of truth for pricing data
- No pricing tables shipped to the browser
- Backend can signal which models used fallback pricing (`has_estimated_pricing`)
- Aggregation at the SQL+Rust layer keeps response payloads small (buckets, not raw rows)

### Non-Paginated Response for Time-Series

The cost-trend endpoint returns a flat `CostTrendResponse` (not `PaginatedResponse<T>`) because time-series data is pre-aggregated — bucket counts are bounded by the time range (max ~720 hourly buckets for 30 days).

### Route Ordering for Static vs Wildcard

`/llm-calls/cost-trend` must be registered BEFORE `/llm-calls/{id}` in `mod.rs` to avoid the wildcard capturing "cost-trend" as an ID parameter. This follows the existing precedent documented in the route registration code.

## Pitfalls

1. **Agent key erasure bug**: Initially, the aggregation query erased `agent_id` to `None` when an `agent_id` filter was active, causing the "By Agent" chart to show all costs as "unknown". Fix: always preserve `agent_id` in the bucket key — let the frontend decide whether to show the toggle.

2. **React purity with Date.now()**: Computing a default `from` using `Date.now()` inside render violates React's purity rule (eslint `react-hooks/purity`). Fix: let the server handle the default — the endpoint already defaults to 24h when `from` is omitted.

3. **recharts + dark theme**: recharts doesn't natively consume CSS custom properties. Chart colors must be hardcoded hex values matching the design system tokens, not Tailwind classes. The chart palette (`CHART_PALETTE`) mirrors the hue order of `agentColors.ts` but as hex values.

## Dependencies

- `recharts` added to `dashboard/package.json` — first charting library in the dashboard
- No Rust dependency changes
