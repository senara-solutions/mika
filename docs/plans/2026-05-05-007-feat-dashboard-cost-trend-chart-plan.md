---
title: "feat: Add CostTrendChart time-series visualization to dashboard"
type: feat
status: active
date: 2026-05-05
issue: 660
---

# feat: Add CostTrendChart time-series visualization to dashboard

## Overview

Add the first time-series chart to the Mika observability dashboard — a `<CostTrendChart>` component that visualizes LLM API cost over time, aggregated from the `llm_calls` SQLite table. The chart supports two variants: single-line (total cost) and stacked-by-agent. This is the lead deliverable for the (C) Hybrid stance decided in #660: 2-3 charts that earn their pixels, not full Grafana.

## Problem Frame

The dashboard is entirely table-based. For an observability platform, cost-over-time trends are the highest-signal chart — they answer "am I spending more or less than usual?" at a glance, which tables cannot. The same chart shape will be reused later for tool-failure-rate-over-time (separate ticket).

## Requirements Trace

- R1. Render a cost-over-time line chart on the LLM Calls page
- R2. Support two variants: single-line (total) and stacked-by-agent
- R3. Aggregate cost server-side from `llm_calls` token counts with model pricing
- R4. Account for cache tokens (cache_read at ~10% input rate, cache_write at ~125%)
- R5. Auto-select time bucket granularity (hourly for <3d, daily for ≥3d)
- R6. Integrate with existing `<TimeRangeFilter>` and `<AgentFilter>` URL params
- R7. Follow Luminescent Core / Midnight Obsidian design system
- R8. Use lifecycle state primitives (`LoadingState`, `ErrorState`, `EmptyState`) from `@senara-solutions/ui`
- R9. Provide ARIA-accessible data fallback

## Scope Boundaries

- **Out of scope:** Token-totals chart, latency p50/p95/p99, agent activity heatmap, dev-run outcome trend (per issue decision)
- **Out of scope:** New "Overview" landing page — chart integrates on the existing LLM Calls page
- **Out of scope:** Real-time polling / auto-refresh — chart follows existing stale-time pattern
- **Out of scope:** Design system rulebook extension for chart grammar — the rulebook is owned by Vincent and updated via direct commits; this PR applies existing token vocabulary to charts

### Deferred to Separate Tasks

- Tool-failure-rate-over-time chart: reuses the same chart shape, file separately
- Comprehensive model pricing database: initial implementation covers major providers with a generous fallback; expanding to all models is iterative

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/dashboard.rs` — handler pattern: `Query<T>` deserialization → `resolve_pagination` → filters struct → `state.dashboard_db.<method>().await` → `Json(response)`
- `crates/mika-agent/src/db.rs` — `LlmCallRow` (line ~456), `LlmCallFilters` (line ~505), `llm_calls` CREATE TABLE (line ~1439)
- `crates/mika-agent/src/async_db.rs` — `AsyncDatabase` closure-based dispatch pattern
- `dashboard/src/api/client.ts` — `apiFetch<T>(path, params)` with auth token injection
- `dashboard/src/api/llmCalls.ts` — existing `useLlmCalls` hook pattern
- `dashboard/src/pages/LlmCalls.tsx` — target page for chart integration
- `packages/ui/src/components/TimeRangeFilter.tsx` — ISO 8601 emission, URL-state friendly
- `packages/ui/src/utils/agentColors.ts` — stable agent color mapping
- `packages/ui/src/theme.css` — design tokens (bg: `#0d0f12`, card: `#151820`, accent: `#7c6af7`, muted: `#a0a8b8`)
- Index: `idx_llm_calls_agent_created ON llm_calls(agent_id, created_at)` — usable for filtered queries

### Institutional Learnings

- **SQLite datetime format** (`docs/solutions/database-issues/sqlite-datetime-format-mismatch.md`): Use `substr(created_at, 1, N)` for bucketing, not `strftime()`, to avoid the `T` vs space format mismatch
- **Dashboard time-range filter pattern** (`docs/solutions/best-practices/dashboard-time-range-filter-full-stack-pattern-2026-04-27.md`): `from`/`to` as ISO 8601 TEXT, lexicographic WHERE comparison
- **DB scoping** (`docs/solutions/656-core-memory-actionable-dashboard-patterns.md`): Use `state.dashboard_db` for cross-agent queries — the cost-trend endpoint is cross-agent by nature
- **Column selection** (`docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`): Use explicit column selection in aggregation queries, not `SELECT *`
- **Filter primitives** (`docs/solutions/655-filter-primitives-unification.md`): Never hand-roll filters; use `@senara-solutions/ui` primitives

### External References

- [recharts documentation](https://recharts.org/en-US/) — React charting library, composable, dark-theme friendly
- Anthropic cache pricing: cache_read at 10% of input rate, cache_write at 125% of input rate

## Key Technical Decisions

- **Server-side cost computation**: Cost computed in the Rust aggregation query, not client-side. Single source of truth for pricing, no pricing data shipped to browser. Backend returns `cost_usd` per bucket.
- **Pricing module placement**: New `crates/mika-agent/src/pricing.rs` module with a `ModelPricing` struct and `estimate_cost()` function. Covers major Anthropic, OpenAI, DeepSeek, Google, Mistral, and Groq models. Unknown models use a conservative fallback ($1.00/$5.00 per MTok). The response signals which models used fallback pricing.
- **Cache token handling**: Full 4-component cost formula: `(input_tokens - cache_read_tokens) * input_rate + cache_read_tokens * (input_rate * 0.1) + cache_write_tokens * (input_rate * 1.25) + output_tokens * output_rate`. Provider-specific multipliers where known.
- **recharts as charting library**: Added to `dashboard/package.json`. Chosen for: React-native composable API, good dark theme support via props, tree-shakeable, no separate CSS. ~55KB gzipped for AreaChart/LineChart. Alternatives (visx, d3, lightweight-charts) rejected: visx requires more boilerplate for basic charts; d3 is not React-native; lightweight-charts is finance-focused.
- **Chart on LLM Calls page, not a new page**: Avoids adding a new route/nav item for a single chart. The chart sits in a card above the table. Natural home — cost is a property of LLM calls.
- **Auto-bucket server-side**: The backend computes bucket granularity from the `from`/`to` span. `<3 days → hourly`, `≥3 days → daily`. Response includes `bucket_size` so the frontend can label the x-axis. Client may override with `bucket=hour|day`.
- **Chart default time range**: When `from` is unset, the chart endpoint defaults to last 24 hours. This is independent of the table filter (table continues to show all data paginated). A subtle label on the chart indicates the effective range.
- **Non-paginated response**: The cost-trend endpoint returns a flat array (not `PaginatedResponse<T>`). Time-series data is pre-aggregated; bucket counts are bounded by the time range (max ~720 hourly buckets for 30 days).

## Open Questions

### Resolved During Planning

- **Where does the pricing table live?** → New `crates/mika-agent/src/pricing.rs` production module. The test-only `cost.rs` in `tests/eval/` is not suitable for runtime use.
- **How should cache tokens factor into cost?** → Full 4-component formula with provider-specific multipliers. Labeled "estimated" since pricing evolves.
- **Should the chart auto-refresh?** → No. Follows existing TanStack React Query stale-time pattern. Users can manually refresh.
- **Agent names vs IDs in legend?** → Use `agent_id` directly — already human-readable in this system (e.g., `mika-dev`, `mika-qa`).
- **Single data point rendering?** → recharts handles single points fine (renders a dot). The tooltip still works.

### Deferred to Implementation

- Exact set of models in the pricing table — expand iteratively based on providers observed in real data
- Whether a compound SQL index is needed for stacked-by-agent performance — test with real data volume first; existing `idx_llm_calls_agent_created` may suffice

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
┌─────────────────────────────────────────────────────────────────┐
│                    LLM Calls Page                                │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Filter Bar: [AgentFilter] [ModelFilter] [TimeRange] [...]│  │
│  └───────────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ CostTrendChart Card                                       │  │
│  │  ┌─ header: "Cost Trend" + variant toggle ──────────────┐│  │
│  │  │ [Total ▪] [By Agent ▪]                               ││  │
│  │  ├──────────────────────────────────────────────────────┤│  │
│  │  │          ╱\                                          ││  │
│  │  │        ╱    \      ╱──\                              ││  │
│  │  │  ──╱\/        ╲╱╱      \──                           ││  │
│  │  │  Mon   Tue   Wed   Thu   Fri                         ││  │
│  │  └──────────────────────────────────────────────────────┘│  │
│  └───────────────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Existing LLM Calls Table                                  │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

**Data flow:**

```
TimeRangeFilter ──(from/to URL params)──► useCostTrend hook
AgentFilter ────(agent_id URL param)────►      │
                                                ▼
                                    GET /api/v1/llm-calls/cost-trend
                                           ?from=...&to=...&agent_id=...
                                                │
                                                ▼
                                    dashboard.rs handler
                                    → db.rs: query_cost_trend()
                                    → pricing.rs: estimate_cost()
                                                │
                                                ▼
                                    { buckets: [...], bucket_size, pricing_info }
                                                │
                                                ▼
                                    <CostTrendChart data={...} variant="total|agent" />
```

## Implementation Units

- [x] **Unit 1: Add pricing module (Rust)**

**Goal:** Create a production pricing module that computes estimated LLM call cost from token counts and model identity.

**Requirements:** R3, R4

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/pricing.rs`
- Modify: `crates/mika-agent/src/lib.rs` (add `pub mod pricing;`)
- Test: `crates/mika-agent/src/pricing.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- `ModelPricing` struct: `input_per_mtok`, `output_per_mtok`, `cache_read_multiplier` (default 0.1), `cache_write_multiplier` (default 1.25)
- `get_pricing(provider: &str, model: &str) -> ModelPricing` — lookup by `(provider, model)` with normalization (strip provider prefixes, case-insensitive). Returns `FALLBACK_PRICING` for unknown models.
- `estimate_call_cost(pricing: &ModelPricing, input_tokens: u64, output_tokens: u64, cache_read_tokens: Option<u64>, cache_write_tokens: Option<u64>) -> f64` — the 4-component formula
- Cover Anthropic (Opus 4, Sonnet 4/4.6, Haiku 4.5), OpenAI (GPT-4o, GPT-4o-mini, o3-mini), DeepSeek (v3, R1), Google (Gemini 2.5 Flash/Pro), Groq (Llama 3.x), Mistral (Large, Small). Ollama/local models get zero cost.
- `FALLBACK_PRICING`: `$1.00/$5.00` per MTok. `is_fallback_pricing()` helper for frontend signaling.

**Patterns to follow:**
- `crates/mika-agent/tests/eval/kg_provider_eval/cost.rs` for the lookup pattern (but this is the production version with full coverage)
- Standalone module pattern like `crates/mika-agent/src/timestamp.rs`

**Test scenarios:**
- Happy path: Anthropic Sonnet 4.6 with known pricing returns expected cost
- Happy path: Full 4-component cost with cache tokens returns correct amount
- Edge case: Zero tokens returns $0.00
- Edge case: Unknown model returns fallback pricing and `is_fallback_pricing` is true
- Edge case: Provider prefix normalization — `openrouter/anthropic/claude-sonnet-4.6` resolves to Anthropic pricing
- Edge case: cache_read_tokens = None treated as 0
- Edge case: Ollama/local provider returns zero cost

**Verification:**
- `cargo test -p mika-agent pricing` passes
- `cargo clippy` clean

---

- [x] **Unit 2: Add cost-trend aggregation query (Rust DB layer)**

**Goal:** Add a SQL aggregation query that returns cost-over-time buckets from the `llm_calls` table, with the pricing module applied per-row.

**Requirements:** R3, R5

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (add `CostTrendBucket`, `CostTrendFilters`, `query_cost_trend()`)
- Modify: `crates/mika-agent/src/async_db.rs` (add async wrapper `query_cost_trend()`)
- Test: `crates/mika-agent/src/db.rs` (inline tests)

**Approach:**
- `CostTrendFilters`: `agent_id: Option<String>`, `model: Option<String>`, `from: Option<String>`, `to: Option<String>`, `bucket: Option<String>` (hour/day/auto, default auto)
- SQL query selects: `substr(created_at, 1, 13) || ':00:00Z'` for hourly buckets, `substr(created_at, 1, 10) || 'T00:00:00Z'` for daily. GROUP BY bucket, agent_id. Returns raw token columns per group.
- Rust-side post-processing: apply `pricing::estimate_call_cost()` per row, aggregate into `CostTrendBucket` structs
- `CostTrendBucket`: `timestamp: String`, `cost_usd: f64`, `input_tokens: u64`, `output_tokens: u64`, `call_count: u64`, `agent_id: Option<String>` (None for total mode)
- `CostTrendResponse`: `buckets: Vec<CostTrendBucket>`, `bucket_size: String`, `has_estimated_pricing: bool`, `estimated_models: Vec<String>`
- Auto-bucket logic: if `from` and `to` span < 3 days → hourly; otherwise → daily. If no `from`, default to `now - 24h`.
- Use `dashboard_db` (cross-agent), not per-agent DB
- Explicit column selection: only `created_at, agent_id, provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens`

**Patterns to follow:**
- `query_llm_calls()` in `db.rs` for the WHERE clause construction pattern with optional filters
- `query_timeline()` for the non-paginated response pattern

**Test scenarios:**
- Happy path: 3 rows across 2 hourly buckets → 2 buckets with correct aggregated cost
- Happy path: Stacked by agent — 2 agents in same bucket → 2 rows per bucket
- Edge case: No rows matching filters → empty buckets array
- Edge case: Auto-bucket selects hourly for 2-day range, daily for 7-day range
- Edge case: No `from` → defaults to last 24h
- Edge case: `from` without `to` → `to` defaults to now
- Edge case: Unknown model in data → `has_estimated_pricing: true`, model listed in `estimated_models`
- Integration: `substr()` bucketing produces valid ISO 8601 timestamps

**Verification:**
- `cargo test -p mika-agent` passes (DB tests)
- Query returns correct aggregation against test data

---

- [x] **Unit 3: Add cost-trend HTTP endpoint (Rust server)**

**Goal:** Wire the cost-trend query into an Axum handler at `GET /api/v1/llm-calls/cost-trend`.

**Requirements:** R3, R5

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/server/dashboard.rs` (add `CostTrendQuery`, `handle_cost_trend()`)
- Modify: `crates/mika-agent/src/server/mod.rs` (add route `.route("/llm-calls/cost-trend", get(dashboard::handle_cost_trend))`)

**Approach:**
- `CostTrendQuery` deserializes: `agent_id`, `model`, `from`, `to`, `bucket`
- Handler builds `CostTrendFilters`, calls `state.dashboard_db.query_cost_trend()`, returns `Json(CostTrendResponse)`
- Route must be registered BEFORE the `/llm-calls/{id}` wildcard route to avoid path conflict

**Patterns to follow:**
- `handle_llm_calls()` handler pattern in `dashboard.rs`
- Route ordering precedent: "Static route MUST come before wildcard" comment in `mod.rs`

**Test scenarios:**
- Happy path: GET with `from`/`to` returns JSON with `buckets` array and `bucket_size`
- Edge case: No query params → defaults applied (last 24h, auto bucket)
- Error path: DB error returns 500 with error message

**Verification:**
- `cargo build` succeeds
- Route is accessible (verified in integration or manual test)

---

- [x] **Unit 4: Add recharts dependency and CostTrendChart component (Frontend)**

**Goal:** Create the `<CostTrendChart>` React component with single-line and stacked-by-agent variants, styled for Midnight Obsidian.

**Requirements:** R1, R2, R7, R8, R9

**Dependencies:** Unit 3

**Files:**
- Modify: `dashboard/package.json` (add `recharts` dependency)
- Create: `dashboard/src/components/CostTrendChart.tsx`
- Create: `dashboard/src/components/CostTrendChart.test.tsx`

**Approach:**
- Add `recharts` to `dashboard/package.json` dependencies
- Component props: `data: CostTrendBucket[]`, `variant: 'total' | 'agent'`, `bucketSize: string`, `isLoading: boolean`, `error: Error | null`, `onRetry: () => void`, `hasEstimatedPricing?: boolean`
- **Total variant**: `<AreaChart>` with single area using `accent` color (`#7c6af7`), gradient fill fading to transparent
- **Agent variant**: `<AreaChart>` with stacked areas, one per unique `agent_id`, colors from `agentColors` utility
- Tooltip: dark-themed custom tooltip showing timestamp, cost formatted as `$X.XX`, token counts
- X-axis: formatted timestamps (hour or date based on `bucketSize`)
- Y-axis: cost in USD with `$` prefix
- Card wrapper: `bg-bg-card rounded-xl p-6` (no 1px borders per luminescent-core "No-Line Rule")
- Header: "Cost Trend" title with variant toggle (two small buttons)
- Lifecycle states: `isLoading ? <LoadingState variant="list" /> : error ? <ErrorState ... /> : empty ? <EmptyState ... /> : chart`
- Estimated pricing footnote when `hasEstimatedPricing` is true
- ARIA: `role="img"` with `aria-label` describing the data range; hidden `<table>` for screen readers

**Patterns to follow:**
- `packages/ui/src/theme.css` design tokens for colors
- `packages/ui/src/utils/agentColors.ts` for agent color mapping
- Lifecycle state pattern from `packages/ui/CLAUDE.md`

**Test scenarios:**
- Happy path: Renders AreaChart with data in total variant
- Happy path: Renders stacked areas per agent in agent variant
- Edge case: Empty data array renders `<EmptyState>`
- Edge case: Loading state renders `<LoadingState>`
- Edge case: Error state renders `<ErrorState>` with retry button
- Edge case: Single bucket renders without crash
- Happy path: Variant toggle switches between total and agent views
- Happy path: Estimated pricing footnote visible when `hasEstimatedPricing` is true

**Verification:**
- `npm run test --prefix dashboard` passes
- Component renders in dev server without errors
- Chart uses design system colors, not default recharts palette

---

- [x] **Unit 5: Add cost-trend API hook and integrate on LLM Calls page (Frontend)**

**Goal:** Wire the chart into the LLM Calls page with data fetching and filter integration.

**Requirements:** R1, R6, R8

**Dependencies:** Unit 4

**Files:**
- Modify: `dashboard/src/api/llmCalls.ts` (add `CostTrendResponse`, `CostTrendBucket`, `useCostTrend` hook)
- Modify: `dashboard/src/pages/LlmCalls.tsx` (add chart above table)

**Approach:**
- `useCostTrend(filters: { agent_id?, model?, from?, to? })` — `useQuery` wrapping `apiFetch('/llm-calls/cost-trend', filters)` with query key `['cost-trend', filters]`
- Chart reads `from`/`to`/`agent_id` from existing URL search params (same source as the table)
- Chart variant state stored in URL param `chart` (`total` | `agent`, default `total`)
- Chart card placed between the filter bar and the table
- When no `from` param is set, the chart query passes `from` as 24h ago (chart-specific default), while the table query remains unfiltered

**Patterns to follow:**
- `useLlmCalls` hook pattern in `dashboard/src/api/llmCalls.ts`
- `useSearchParamsFilter` hook for URL-state management
- Page composition pattern from existing list pages

**Test scenarios:**
- Happy path: LLM Calls page renders chart above table
- Happy path: Changing time range filter updates both chart and table
- Happy path: Selecting an agent filters chart data
- Edge case: No `from` param → chart defaults to 24h, table shows all
- Happy path: Chart variant toggle persists in URL param

**Verification:**
- LLM Calls page loads with chart visible in dev server
- Filters control both chart and table
- URL reflects chart variant state

## System-Wide Impact

- **Interaction graph:** Chart data flows through the same `dashboard_db` connection as existing list queries. The periodic WAL checkpoint (every 60s) ensures fresh data. No new write paths.
- **Error propagation:** Chart fetch failures are isolated to the chart card — the table continues to work independently (separate `useQuery`). `<ErrorState>` with retry.
- **State lifecycle risks:** None — chart is read-only, no mutations. The `useCostTrend` query is independent of the `useLlmCalls` pagination query.
- **API surface parity:** The new endpoint follows the same auth pattern (dashboard or internal token). It does not affect existing endpoints.
- **Unchanged invariants:** Existing `/api/v1/llm-calls` endpoint, `LlmCallRow` type, `LlmCallFilters`, and all other dashboard pages are unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| recharts bundle size (~55KB gzip) increases dashboard load | Tree-shake via Vite; only import AreaChart, XAxis, YAxis, Tooltip, ResponsiveContainer — not the full library |
| Pricing estimates diverge from actual provider billing | Label as "estimated cost"; surface `has_estimated_pricing` flag; expand pricing table iteratively |
| SQLite `substr()` bucketing performance on large datasets | Existing `idx_llm_calls_agent_created` index covers agent-filtered queries; unfiltered queries over 100K+ rows may need a dedicated index — defer until measured |
| Route ordering conflict: `/llm-calls/cost-trend` vs `/llm-calls/{id}` | Register the static route before the wildcard, following the existing precedent in `mod.rs` |

## Sources & References

- Related issue: #660
- Related milestone: #13 (Dashboard improvements)
- Stitch designs: project `6562713725762717689`, screens `5b65fb5f76dd4edaa3e4e72a9c00ee13` and `2443ccdf05d44295ade2f7c17574d7c3`
- Design system: `docs/design/luminescent-core.md`, `docs/design/north-star.md`
- Pricing reference: Anthropic API pricing page, OpenAI API pricing page
