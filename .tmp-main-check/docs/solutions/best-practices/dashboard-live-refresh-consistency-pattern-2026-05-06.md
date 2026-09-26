---
title: Dashboard live-refresh consistency pattern
date: 2026-05-06
category: best-practices
module: dashboard
problem_type: best_practice
component: frontend_stimulus
severity: medium
applies_when:
  - Adding auto-refresh to a new dashboard page
  - Creating a detail page for an entity with active/terminal lifecycle
  - Adding polling to a React Query hook
tags:
  - dashboard
  - live-refresh
  - react-query
  - polling
  - refetch-interval
  - auto-refresh
  - packages-ui
---

# Dashboard live-refresh consistency pattern

## Context

The dashboard had inconsistent live-refresh behavior — only Event Timeline and Home had auto-refresh, while operators stared at stale Dev Run and Team Run detail pages during active autonomous runs. Each implementation was hand-rolled with different toggle markup, intervals, and guard logic.

## Guidance

Use the standardized two-primitive pattern for all live-refresh surfaces:

1. **`<LiveRefreshToggle />`** from `@senara-solutions/ui` — pure presentational component (toggle switch + LIVE badge). Props: `{ isLive, onToggle, disabled?, className? }`.

2. **`useLiveRefresh()`** hook from `dashboard/src/hooks/useLiveRefresh.ts` — manages toggle state and computes `refetchInterval`. Accepts `{ defaultEnabled?, interval?, isDefaultView? }`, returns `{ isLive, toggle, refetchInterval, isEffectivelyLive }`.

**Page classification determines defaults:**

| Page type | `defaultEnabled` | `interval` | `isDefaultView` semantics |
|-----------|-----------------|-----------|--------------------------|
| Detail (active entity) | `true` | `5_000` | Entity is non-terminal (`!TERMINAL_STATUSES.has(status)`) |
| List (user-toggleable) | `false` | `15_000` | No filters active AND page === 1 |
| Static (completed entity) | N/A — no polling | N/A | N/A |

**API hooks accept `refetchInterval` as an optional parameter:**

```typescript
// Pattern: add refetchInterval + placeholderData to any hook that may be polled
import { useQuery, keepPreviousData } from '@tanstack/react-query'

export function useDevRun(taskId: string | undefined, refetchInterval?: number | false) {
  return useQuery<DevRun>({
    queryKey: ['dev-run', taskId],
    queryFn: () => apiFetch(`/dev-runs/${taskId}`),
    enabled: !!taskId,
    refetchInterval,
    placeholderData: keepPreviousData, // prevents loading-state flicker during refetch
  })
}
```

**Detail page circular dependency pattern:**

Detail pages have a hook ordering issue: `useDevRun` needs `refetchInterval`, but `refetchInterval` depends on `run.status` from `useDevRun`. Resolve with a `useEffect`-synced state variable:

```typescript
const [pollInterval, setPollInterval] = useState<number | false>(false)
const { data: run } = useDevRun(taskId, pollInterval)

const isActive = !!run && !TERMINAL_STATUSES.has(run.status)
const { isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
  defaultEnabled: true,
  interval: 5_000,
  isDefaultView: isActive,
})

// Sync computed interval → state (one render behind, triggers re-render)
useEffect(() => { setPollInterval(refetchInterval) }, [refetchInterval])
```

This creates a one-render-cycle delay (~16ms) between data load and polling start — imperceptible to users.

## Why This Matters

- **Consistency:** All live-refresh surfaces share the same visual affordance and behavior
- **Design system compliance:** `LiveRefreshToggle` in `packages/ui/` is enforced — hand-rolled toggles are a review fail
- **SQLite safety:** 15s for list pages respects the WAL checkpoint cadence (~60s freshness floor). 5s for detail pages is acceptable for single-entity queries.
- **Tab visibility:** React Query's `refetchIntervalInBackground: false` (default) pauses polling on hidden tabs for free
- **`isDefaultView` guard:** Prevents data-shifting on filtered/paginated views where new rows would push existing ones under the cursor

## When to Apply

- Adding a new dashboard page that shows time-sensitive data
- Converting a static detail page to support live updates
- Adding polling to an existing React Query hook

## Examples

**List page (Sessions):**

```tsx
const isDefaultView = !filters.agent_id && !filters.from && !filters.to && (filters.page ?? 1) === 1
const { isLive, isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
  defaultEnabled: false, interval: 15_000, isDefaultView
})
const { data } = useSessions(filters, refetchInterval)

// In header:
<LiveRefreshToggle isLive={isEffectivelyLive} onToggle={toggle} disabled={!isDefaultView && isLive} />
```

**Detail page (Dev Run):**

```tsx
const [pollInterval, setPollInterval] = useState<number | false>(false)
const { data: run } = useDevRun(taskId, pollInterval)
const isActive = !!run && !TERMINAL_STATUSES.has(run.status)
const { isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
  defaultEnabled: true, interval: 5_000, isDefaultView: isActive
})
useEffect(() => { setPollInterval(refetchInterval) }, [refetchInterval])

// In header:
<LiveRefreshToggle isLive={isEffectivelyLive} onToggle={toggle} />
```

**Do NOT:**
- Hand-roll toggle switch markup — use `<LiveRefreshToggle />`
- Add `refetchInterval` to `useAgents()` — global static data relies on `staleTime` + window-focus refetch
- Modify `useTaskDescendants` — it has its own status-gated polling logic
- Poll GitHub hooks (`useGitHubIssue`, `useGitHubPull`) — they have long `staleTime` and would risk API rate limits

## Related

- `docs/solutions/best-practices/dashboard-landing-page-widget-composition-2026-05-06.md` — Home page 15s polling pattern (always-on, no toggle)
- `docs/solutions/database-issues/dashboard-stale-wal-snapshot-2026-04-27.md` — SQLite WAL freshness constraints
- mika#662 — Live-refresh consistency issue
- mika#13 — Dashboard improvements milestone
