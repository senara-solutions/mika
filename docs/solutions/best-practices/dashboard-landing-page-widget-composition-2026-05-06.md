---
module: dashboard
date: 2026-05-06
problem_type: best_practice
component: tooling
severity: medium
tags:
  - dashboard
  - landing-page
  - widget
  - react-query
  - composition
  - refetch-interval
  - cost-trend-chart
  - empty-state
applies_when:
  - Adding a new dashboard overview or summary page that composes multiple existing API hooks
  - Embedding CostTrendChart or other self-contained components inside WidgetSection wrappers
  - Wiring auto-refresh (refetchInterval) across multiple independent widgets on one page
---

# Dashboard Landing Page — Widget Composition Patterns

## Context

The dashboard landing page (#666) composes six independent data sources (agents, tasks, dev runs, team runs, cost trend, timeline) into a single "state of the world" view. Each widget fetches its own data via React Query and independently handles loading/error/empty states. This raised several practical issues around component nesting, refresh coordination, and the shared hook API.

## Guidance

### 1. Avoid card-within-card nesting when embedding self-contained components

`CostTrendChart` already renders its own `bg-bg-card rounded-xl p-6` container with internal loading/error/empty states. Wrapping it in `WidgetSection` (which also renders `bg-bg-card rounded-2xl p-5`) creates double-card nesting with nested padding and backgrounds.

**Solution:** For components with their own card container, skip `WidgetSection` and render a custom section header inline:

```tsx
<section>
  <div className="flex items-center justify-between mb-2">
    <h3 className="text-[11px] text-muted/60 font-medium uppercase tracking-wider">
      Cost (24h)
    </h3>
    <Link to="/llm-calls" className="...">View all</Link>
  </div>
  <CostTrendChart ... />
</section>
```

### 2. Use inline `useQuery` when hooks don't support `refetchInterval`

Shared hooks like `useTasks()`, `useDevRuns()`, `useTeamRuns()` don't accept a `refetchInterval` parameter — they're designed for list pages without auto-refresh. Rather than modifying shared hooks for a landing-page-only concern, use `useQuery` directly in widget components:

```tsx
const { data, isLoading, error, refetch } = useQuery<PaginatedResponse<TaskItem>>({
  queryKey: ['tasks', filters],
  queryFn: () => apiFetch('/tasks', filters),
  refetchInterval,  // passed as prop from the parent page
})
```

This reuses the same query keys as the shared hooks, so React Query deduplicates across pages.

### 3. `useAgents()` is a special case — unpaginated and shared

`useAgents()` returns `Agent[]` (not `PaginatedResponse<T>`) and doesn't accept options. It's globally cached on `['agents']`. Multiple callsites (Home gate + AgentsSummaryWidget) produce only one network request via React Query dedup. Don't modify it to accept `refetchInterval` — instead rely on the global `staleTime` and window-focus refetch.

### 4. Use a page-level agents gate for fresh-install UX

When `useAgents()` returns empty, show a single page-level `<EmptyState>` rather than six independent empty widgets — the stacked empty states are confusing for a new user. Gate the entire widget grid on the agents query:

```tsx
{agentsLoading ? (
  <LoadingState variant="list" rows={6} />
) : agentsError ? (
  <ErrorState message={formatApiError(agentsError)} retry={() => agentsRefetch()} />
) : !agents || agents.length === 0 ? (
  <EmptyState title="Welcome to Mika" message="No agents provisioned." icon={<Bot size={32} />} />
) : (
  <div className="space-y-4">
    {/* all widgets */}
  </div>
)}
```

### 5. Shared refetch interval as a constant, not a global setting

Define `HOME_REFETCH_INTERVAL = 15_000` as a constant in `Home.tsx` and pass it as a prop to each widget. 15s is a safe default for six parallel hooks against SQLite (~24 req/min). Don't use React Query's global `refetchInterval` — it would affect all pages.

## Why This Matters

- Double-card nesting creates visual artifacts (nested borders, doubled padding) that break the Luminescent Core design system
- Modifying shared hooks for page-specific concerns couples unrelated surfaces
- Six empty-state blocks on fresh install look broken, not welcoming
- Aggressive polling (5s × 6 hooks = 72 req/min) risks SQLite contention on single-threaded backends

## When to Apply

- Adding new dashboard overview/summary pages with multiple composed widgets
- Embedding existing chart/visualization components that have their own containers
- Wiring auto-refresh on pages with many independent data sources
- Handling zero-data states on aggregate pages

## Examples

The landing page at `dashboard/src/pages/Home.tsx` demonstrates all patterns above. Widget components at `dashboard/src/components/home/` show the inline `useQuery` pattern for `refetchInterval` support.
