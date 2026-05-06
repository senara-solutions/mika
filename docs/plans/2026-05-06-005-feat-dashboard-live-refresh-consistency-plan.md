---
title: "feat: Dashboard live-refresh consistency across time-sensitive pages"
type: feat
status: active
date: 2026-05-06
---

# feat: Dashboard live-refresh consistency across time-sensitive pages

## Overview

Extract the Event Timeline's live-refresh pattern (LIVE badge + auto-refresh toggle + polling) into shared primitives and apply them consistently to all time-sensitive dashboard pages. Today, only Event Timeline and Home have any form of auto-refresh — operators stare at stale data during active dev runs, team runs, and session activity.

## Problem Frame

The Event Timeline page has a working LIVE badge and auto-refresh toggle with 5s polling (gated by `isDefaultView`). The Home page has always-on 15s polling with no toggle. No other dashboard page has any live-refresh capability.

Pages that would benefit:
- **Dev Run detail** — active runs progress through pipeline stages (plan → work → PR → QA → merge) over minutes
- **Team Run detail** — active runs iterate through agent assignments
- **Sessions list** — new sessions arrive from webhooks during active runs
- **Tasks list** — status transitions on in-flight work items
- **LLM Calls list** — new calls stream in during active runs

The operator's primary workflow during autonomous dev runs is watching the detail page — manual refresh creates friction and uncertainty about whether the system is still working.

## Requirements Trace

- R1. `<LiveRefreshToggle />` component extracted to `packages/ui/` as a pure presentational primitive
- R2. `useLiveRefresh()` hook in `dashboard/src/hooks/` managing toggle state, `isDefaultView` guard, and resolved `refetchInterval`
- R3. In-flight detail pages (Dev Run, Team Run) auto-refresh by default when status is non-terminal
- R4. List pages (Sessions, Tasks, LLM Calls) have user-toggleable live refresh (off by default)
- R5. Polling pauses on hidden tabs (React Query default — verify, don't re-implement)
- R6. Timeline page refactored to use the new shared primitives (no behavior change)
- R7. Home page unchanged (always-on, no toggle — different pattern)

## Scope Boundaries

- No WebSocket/SSE — polling only (infrastructure doesn't support push yet)
- No new backend endpoints — uses existing API hooks with `refetchInterval`
- Session detail page is **out of scope** — sessions don't have a clear "active/completed" status field. The issue mentions "Session currently active" but `SessionDetail` only has `ended_at` (nullable) which is not reliably set during activity
- No error-count auto-pause — React Query's built-in `retry: 1` plus `refetchInterval` continuation is sufficient. A future issue can add consecutive-failure pausing if operators report noise
- Home page keeps its current always-on 15s pattern — it uses a different interaction model (no toggle, always live)

### Deferred to Separate Tasks

- Session detail live-refresh: requires defining session "active" status semantics first
- Error-count based auto-pause: track separately if polling noise becomes a problem in practice

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/pages/Timeline.tsx` lines 19, 44-71 — canonical toggle + LIVE badge + `autoRefresh` state
- `dashboard/src/api/timeline.ts` lines 25-41 — `useTimeline()` with `refetchInterval` gated on `autoRefresh && isDefaultView`
- `dashboard/src/pages/Home.tsx` lines 12, 25 — `HOME_REFETCH_INTERVAL = 15_000`, always-on LIVE badge
- `dashboard/src/api/tasks.ts` lines 92-108 — `useTaskDescendants()` with `parentStatus`-gated conditional `refetchInterval` callback
- `packages/ui/src/components/StatusBadge.tsx` — `variant="success" label="Live" dotPulse` for LIVE indicator
- `dashboard/src/hooks/useSearchParamsFilter.ts` — URL-state filter management used by all list pages
- `packages/ui/CLAUDE.md` — enforcement table pattern for canonical primitives

### Institutional Learnings

- **Widget composition (2026-05-06):** Shared hooks (`useTasks()`, `useDevRuns()`) do NOT accept `refetchInterval` — they were designed for list pages without auto-refresh. Home page uses inline `useQuery` with identical query keys for dedup. The same pattern should apply here: add `refetchInterval` parameter to existing hooks rather than creating inline queries.
- **WAL snapshot staleness (2026-04-27):** SQLite WAL mode has a ~60s data freshness floor due to periodic `PRAGMA wal_checkpoint(PASSIVE)`. Polling faster than 15s provides diminishing returns for list pages.
- **TUI polling removal:** Polling should match expected change frequency. Detail pages change during active runs (high frequency → 5s). List pages have lower change frequency (15s).
- **State catalog (2026-04-27):** The `isLoading ? <LoadingState> : error ? <ErrorState> : empty ? <EmptyState> : content` ternary is canonical and must be preserved. `placeholderData: keepPreviousData` should be added to polled queries so refetches don't flash loading state.
- **Wrapper-component constraint (luminescent-core §248):** No `<QueryStates />` wrapper that reads query-library state in `packages/ui/`. The `useLiveRefresh` hook must live in `dashboard/src/hooks/`, not in the shared library.

## Key Technical Decisions

- **`LiveRefreshToggle` in `packages/ui/`, `useLiveRefresh` in dashboard:** The toggle is purely presentational (switch + LIVE badge) and belongs in the shared library. The hook manages React Query-adjacent state (computing `refetchInterval`) and must stay in the dashboard per the wrapper-component constraint.
- **Add `refetchInterval` parameter to existing API hooks:** Rather than creating inline `useQuery` calls (Home page pattern), extend `useDevRun()`, `useTeamRun()`, etc. to accept an optional `refetchInterval` parameter. This keeps query keys stable and avoids duplication. The parameter defaults to `undefined` (no polling) preserving backward compatibility.
- **5s for detail pages, 15s for list pages:** Detail pages show a single entity whose status the operator is actively watching — 5s matches Timeline precedent. List pages multiplex several rows and hit SQLite harder — 15s matches Home widget precedent and respects WAL checkpoint cadence.
- **Replicate `isDefaultView` guard for list pages:** Polling on filtered/paginated views causes "data shifting under cursor" (new items push existing rows down mid-click). Following Timeline's pattern: polling is active only when on page 1 with no filters set. The toggle is still visible but the LIVE badge shows "Paused" state when the guard suppresses polling.
- **Status-gated polling for detail pages:** Pass `status` from the primary hook (`useDevRun`, `useTeamRun`) into secondary hooks. Secondary hooks check `TERMINAL_STATUSES` to stop polling when the entity completes. Follows the existing `useTaskDescendants(rootTaskId, parentStatus)` pattern.
- **`placeholderData: keepPreviousData` on polled queries:** Prevents loading-state flicker during refetch cycles. Standard React Query pattern for polling UIs.

## Open Questions

### Resolved During Planning

- **Where does `LiveRefreshToggle` live?** In `packages/ui/` — it's a pure presentational component (toggle switch + LIVE badge composition). The hook stays in dashboard.
- **Should list page polling respect `isDefaultView`?** Yes — matches Timeline precedent and prevents data-shifting UX issues on filtered/paginated views.
- **Should Session detail page auto-refresh?** No — sessions lack a reliable "active" status field. Deferred to a separate task.

### Deferred to Implementation

- **Exact `isDefaultView` computation per page:** Each list page has different filter fields. The `useLiveRefresh` hook should accept a generic `isDefaultView` boolean computed by the page, rather than computing it internally.
- **CSS transition on LIVE badge appear/disappear:** Detail pages should fade the LIVE badge smoothly when status transitions to terminal. Exact transition timing TBD during implementation.

## Implementation Units

- [ ] **Unit 1: Extract `<LiveRefreshToggle />` to `packages/ui/`**

  **Goal:** Create a reusable presentational component that composes the switch toggle + LIVE badge pattern currently hand-rolled in Timeline.tsx.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Create: `packages/ui/src/components/LiveRefreshToggle.tsx`
  - Modify: `packages/ui/src/index.ts` (add export)
  - Modify: `packages/ui/CLAUDE.md` (add to enforcement table)
  - Test: `packages/ui/src/components/__tests__/LiveRefreshToggle.test.tsx`

  **Approach:**
  - Component accepts `{ isLive: boolean, onToggle: () => void, disabled?: boolean, className?: string }`
  - When `isLive` is true: renders `<StatusBadge variant="success" label="Live" dotPulse />` + toggle switch in ON position
  - When `isLive` is false: renders only the toggle switch in OFF position (no badge)
  - When `disabled` is true: toggle is visually muted and non-interactive (for `isDefaultView` suppression)
  - Extracts the exact toggle switch markup from Timeline.tsx lines 54-69 (the `<button role="switch">` with `aria-checked`)
  - Wraps both pieces in a flex container with label "Auto-refresh"

  **Patterns to follow:**
  - `packages/ui/src/components/StatusBadge.tsx` — component structure, default export
  - `packages/ui/src/components/AgentFilter.tsx` — thin composition pattern (delegates to StatusBadge)
  - Timeline.tsx lines 54-69 — exact toggle switch markup to extract

  **Test scenarios:**
  - Happy path: renders LIVE badge when `isLive=true`, hides badge when `isLive=false`
  - Happy path: toggle switch reflects `aria-checked` matching `isLive` prop
  - Happy path: calls `onToggle` when switch is clicked
  - Edge case: disabled prop prevents click and applies muted styling
  - Happy path: accessible — `role="switch"` and `aria-checked` present

  **Verification:**
  - Component renders in isolation (`npm run build --prefix packages/ui` succeeds)
  - Enforcement table in `packages/ui/CLAUDE.md` updated with new row

- [ ] **Unit 2: Create `useLiveRefresh()` hook in dashboard**

  **Goal:** Encapsulate live-refresh toggle state and `refetchInterval` computation into a reusable hook for all dashboard pages.

  **Requirements:** R2, R5

  **Dependencies:** None (parallel with Unit 1)

  **Files:**
  - Create: `dashboard/src/hooks/useLiveRefresh.ts`
  - Test: `dashboard/src/hooks/__tests__/useLiveRefresh.test.ts`

  **Approach:**
  - Signature: `useLiveRefresh({ defaultEnabled?: boolean, interval?: number, isDefaultView?: boolean })`
  - Returns: `{ isLive: boolean, toggle: () => void, refetchInterval: number | false, isEffectivelyLive: boolean }`
  - `isLive` reflects the user's toggle choice. `isEffectivelyLive` is `isLive && isDefaultView` — the actual polling state accounting for the guard.
  - `refetchInterval` is `isEffectivelyLive ? interval : false`
  - `defaultEnabled` defaults to `false` (list pages). Detail pages pass `true`.
  - `interval` defaults to `15_000` (list pages). Detail pages pass `5_000`.
  - `isDefaultView` is caller-determined — its semantics differ by page type: for list pages it means "no filters active AND page === 1" (prevents data-shifting); for detail pages it means "entity is non-terminal" (stops polling when work completes). The hook treats it as an opaque boolean gate — the JSDoc should document both use cases.
  - Tab visibility pause is handled by React Query's `refetchIntervalInBackground: false` default — no custom implementation needed. The hook's JSDoc should document this.

  **Patterns to follow:**
  - `dashboard/src/hooks/useSearchParamsFilter.ts` — hook file structure
  - `dashboard/src/api/timeline.ts` lines 26-33 — `isDefaultView` guard logic (extracted to caller)

  **Test scenarios:**
  - Happy path: returns `refetchInterval: false` when `defaultEnabled` is false and toggle not activated
  - Happy path: returns `refetchInterval: 15000` (default interval) when toggled on and `isDefaultView` is true
  - Happy path: returns `refetchInterval: false` when toggled on but `isDefaultView` is false
  - Happy path: `toggle()` flips `isLive` state
  - Edge case: `isEffectivelyLive` is false when `isLive` is true but `isDefaultView` is false
  - Happy path: custom interval (5000) is returned when provided and conditions met

  **Verification:**
  - Hook tests pass
  - TypeScript compiles without errors

- [ ] **Unit 3: Add `refetchInterval` parameter to API hooks**

  **Goal:** Extend existing API hooks to accept an optional `refetchInterval` parameter so pages can enable polling without duplicating query logic.

  **Requirements:** R3, R4 (infrastructure for both)

  **Dependencies:** None (parallel with Units 1-2)

  **Files:**
  - Modify: `dashboard/src/api/devRuns.ts` — `useDevRun(taskId, refetchInterval?)`
  - Modify: `dashboard/src/api/teams.ts` — `useTeamRun(runId, refetchInterval?)`, `useTeamRunSummary(runId, refetchInterval?)`, `useTeamWorkspace(runId, refetchInterval?)`
  - Modify: `dashboard/src/api/sessions.ts` — `useSessions(filters, refetchInterval?)`
  - Modify: `dashboard/src/api/tasks.ts` — `useTasks(filters, refetchInterval?)`, `useTaskSessions(taskId, refetchInterval?)`. Export `TERMINAL_STATUSES` for reuse by detail pages.
  - Modify: `dashboard/src/api/llmCalls.ts` — `useLlmCalls(filters, refetchInterval?)`, `useCostTrend(filters, refetchInterval?)`, `useTraceLlmCalls(traceId, refetchInterval?)`
  - Modify: `dashboard/src/api/toolCalls.ts` — `useTraceToolCalls(traceId, refetchInterval?)`
  - Test: `dashboard/src/api/__tests__/devRuns.test.ts`

  **Approach:**
  - Add optional `refetchInterval?: number | false` parameter to each hook
  - Pass through to `useQuery({ ..., refetchInterval })` — `undefined` means "no polling" (React Query default)
  - Add `placeholderData: keepPreviousData` to hooks that will be polled (all modified hooks) — prevents loading-state flicker during refetch. Import `keepPreviousData` from `@tanstack/react-query` (React Query v5 export).
  - Do NOT change existing query keys — polling is orthogonal to caching
  - `useTaskDescendants` already has its own `refetchInterval` logic — leave it unchanged
  - Export `TERMINAL_STATUSES` from `tasks.ts` so DevRunDetail and TeamRunDetail can import it. The set (`completed`, `delivered`, `failed`, `cancelled`) applies to all entity types (tasks, dev runs, team runs share the same status enum).
  - **GitHub hooks excluded from polling:** `useGitHubIssue` and `useGitHubPull` (in `github.ts`) already have `staleTime: 5min` and `2min` respectively. GitHub data changes far less frequently than run status. Polling these at 5s would be redundant and risk GitHub API rate limits. Leave them unchanged — they refresh on window focus via React Query defaults.

  **Patterns to follow:**
  - `dashboard/src/api/timeline.ts` — `useTimeline(filters, enabled, autoRefresh)` with `refetchInterval` in useQuery options
  - `dashboard/src/api/tasks.ts` lines 94-107 — `useTaskDescendants` conditional `refetchInterval`

  **Test scenarios:**
  - Happy path: `useDevRun(id)` without `refetchInterval` behaves identically to current (no polling)
  - Happy path: `useDevRun(id, 5000)` passes `refetchInterval: 5000` to React Query
  - Happy path: `useTasks(filters, 15000)` passes both filters and `refetchInterval`
  - Edge case: `refetchInterval: false` explicitly disables polling

  **Verification:**
  - All existing tests still pass (no behavior change for current callsites)
  - TypeScript compiles — new parameter is optional, existing callers unaffected

- [ ] **Unit 4: Wire live-refresh to Dev Run detail page**

  **Goal:** Dev Run detail auto-refreshes by default when the run is active (non-terminal status), with a toggle to pause.

  **Requirements:** R3

  **Dependencies:** Units 1, 2, 3

  **Files:**
  - Modify: `dashboard/src/pages/DevRunDetail.tsx`
  - Test: `dashboard/src/pages/__tests__/DevRunDetail.test.tsx`

  **Approach:**
  - Import `useLiveRefresh` and `LiveRefreshToggle`
  - Compute `isActive = !!run && !TERMINAL_STATUSES.has(run.status)` from `useDevRun` result
  - Call `useLiveRefresh({ defaultEnabled: true, interval: 5_000, isDefaultView: isActive })` — `isDefaultView` here means "the entity is still active". When run completes, `isActive` becomes false, which stops polling and hides the LIVE badge automatically.
  - Pass `refetchInterval` from hook to `useDevRun(taskId, refetchInterval)`
  - Pass same `refetchInterval` to `useTaskSessions(taskId, refetchInterval)` — secondary hook polls in sync with the primary
  - Do NOT pass `refetchInterval` to `useGitHubIssue`/`useGitHubPull` — GitHub hooks have long `staleTime` and refresh on window focus (see Unit 3 rationale)
  - `useTaskDescendants` already has its own status-gated polling — leave it as-is
  - Render `<LiveRefreshToggle isLive={isEffectivelyLive} onToggle={toggle} />` in the page header alongside the title
  - LIVE badge disappears naturally when status transitions to terminal (no special transition needed — `isActive` goes false → `isEffectivelyLive` goes false → badge hides)

  **Patterns to follow:**
  - `dashboard/src/pages/Timeline.tsx` lines 41-71 — header layout with LIVE badge and toggle
  - `dashboard/src/api/tasks.ts` `TERMINAL_STATUSES` set — reuse for status check

  **Test scenarios:**
  - Happy path: LIVE badge and toggle visible when run status is `in_progress`
  - Happy path: polling active at 5s when run is active and toggle is on
  - Happy path: LIVE badge disappears when run status transitions to `completed`
  - Happy path: toggle pauses polling while run is still active
  - Edge case: navigating to an already-completed run shows no LIVE badge and no polling
  - Integration: `useTaskSessions` receives the same `refetchInterval` as the primary `useDevRun`; GitHub hooks are excluded from polling

  **Verification:**
  - Active dev run page shows LIVE badge and data updates without manual refresh
  - Completed dev run page is static — no polling, no badge

- [ ] **Unit 5: Wire live-refresh to Team Run detail page**

  **Goal:** Team Run detail auto-refreshes by default when the run is active, with a toggle to pause.

  **Requirements:** R3

  **Dependencies:** Units 1, 2, 3

  **Files:**
  - Modify: `dashboard/src/pages/TeamRunDetail.tsx`
  - Test: `dashboard/src/pages/__tests__/TeamRunDetail.test.tsx`

  **Approach:**
  - Same pattern as Unit 4 but with Team Run hooks
  - Compute `isActive = !!run && !TERMINAL_STATUSES.has(run.status)` from `useTeamRun` result
  - Call `useLiveRefresh({ defaultEnabled: true, interval: 5_000, isDefaultView: isActive })`
  - Pass `refetchInterval` to `useTeamRun(runId, refetchInterval)`, `useTeamRunSummary(runId, refetchInterval)`, `useTeamWorkspace(runId, refetchInterval)`
  - Pass `refetchInterval` to `useTraceLlmCalls(traceId, refetchInterval)` and `useTraceToolCalls(traceId, refetchInterval)` (both hooks extended in Unit 3)
  - Render `<LiveRefreshToggle>` in header

  **Patterns to follow:**
  - Unit 4 (DevRunDetail) — identical wiring pattern

  **Test scenarios:**
  - Happy path: LIVE badge visible when team run status is `running`
  - Happy path: polling stops when run transitions to `completed` or `failed`
  - Happy path: toggle pauses/resumes polling
  - Edge case: completed team run shows no LIVE badge
  - Integration: all five hooks receive the same `refetchInterval`

  **Verification:**
  - Active team run page auto-refreshes and shows iteration progress
  - Completed team run page is static

- [ ] **Unit 6: Wire live-refresh to list pages (Sessions, Tasks, LLM Calls)**

  **Goal:** Sessions, Tasks, and LLM Calls list pages gain a user-toggleable auto-refresh (off by default) with the `isDefaultView` guard.

  **Requirements:** R4

  **Dependencies:** Units 1, 2, 3

  **Files:**
  - Modify: `dashboard/src/pages/Sessions.tsx`
  - Modify: `dashboard/src/pages/Tasks.tsx`
  - Modify: `dashboard/src/pages/LlmCalls.tsx`
  - Test: `dashboard/src/pages/__tests__/Sessions.test.tsx`

  **Approach:**
  - Each page: import `useLiveRefresh` and `LiveRefreshToggle`
  - Compute `isDefaultView` from the page's filter state: no filters active AND page === 1. Each page has different filter fields — compute per-page.
  - Call `useLiveRefresh({ defaultEnabled: false, interval: 15_000, isDefaultView })`
  - Pass `refetchInterval` to the page's primary data hook(s)
  - For Tasks page: four sections each call `useTasks(sectionFilters)` — pass `refetchInterval` to all four
  - For LLM Calls page: pass `refetchInterval` to both `useLlmCalls(filters, refetchInterval)` and `useCostTrend(costTrendFilters, refetchInterval)`
  - Render `<LiveRefreshToggle>` in page header, next to the page title (matching Timeline layout)
  - When `isDefaultView` is false and toggle is on, the toggle remains visible but `LiveRefreshToggle` receives `disabled={!isDefaultView}` to indicate polling is suppressed

  **Patterns to follow:**
  - `dashboard/src/pages/Timeline.tsx` — header layout with toggle
  - `dashboard/src/api/timeline.ts` lines 26-33 — `isDefaultView` guard shape

  **Test scenarios:**
  - Happy path: Sessions list shows toggle OFF by default, no LIVE badge
  - Happy path: enabling toggle shows LIVE badge and starts 15s polling on default view
  - Happy path: applying a filter while toggle is ON suppresses polling (LIVE badge hidden or disabled state)
  - Happy path: clearing filters restores polling when toggle is still ON
  - Edge case: navigating past page 1 suppresses polling
  - Happy path: Tasks page — all four sections receive `refetchInterval`
  - Happy path: LLM Calls page — both data and cost-trend hooks receive `refetchInterval`

  **Verification:**
  - Toggle appears on all three list pages
  - Enabling toggle on default view starts visible data refresh
  - Filters suppress polling as expected

- [ ] **Unit 7: Refactor Timeline page to use shared primitives**

  **Goal:** Replace Timeline's hand-rolled toggle and inline `isDefaultView` logic with the new shared `LiveRefreshToggle` + `useLiveRefresh` primitives. No behavior change.

  **Requirements:** R6

  **Dependencies:** Units 1, 2

  **Files:**
  - Modify: `dashboard/src/pages/Timeline.tsx`
  - Modify: `dashboard/src/api/timeline.ts` — simplify `useTimeline` to accept `refetchInterval` directly instead of computing internally
  - Test: `dashboard/src/pages/__tests__/Timeline.test.tsx`

  **Approach:**
  - Replace `useState(true)` + hand-rolled toggle + inline badge with `useLiveRefresh({ defaultEnabled: true, interval: 5_000, isDefaultView })` + `<LiveRefreshToggle>`
  - Compute `isDefaultView` in the page (same logic currently in `useTimeline`)
  - Simplify `useTimeline` to accept `refetchInterval?: number | false` instead of `autoRefresh` boolean — removes the `isDefaultView` computation from the hook
  - Pass `refetchInterval` from `useLiveRefresh` to `useTimeline`
  - This is a pure refactor — behavior must be identical before and after

  **Patterns to follow:**
  - Units 4-6 — same wiring pattern

  **Test scenarios:**
  - Happy path: auto-refresh ON by default with LIVE badge (same as before)
  - Happy path: toggle OFF hides LIVE badge and stops polling (same as before)
  - Happy path: applying filters suppresses polling even when toggle is ON (same as before)
  - Integration: `useTimeline` still receives the correct `refetchInterval` value

  **Verification:**
  - Timeline page behavior is identical to before the refactor
  - No hand-rolled toggle markup remains in Timeline.tsx

## System-Wide Impact

- **Interaction graph:** `LiveRefreshToggle` is consumed by 6 pages (Timeline, DevRunDetail, TeamRunDetail, Sessions, Tasks, LlmCalls). `useLiveRefresh` hook is consumed by the same 6 pages. Changes to the toggle API ripple to all consumers.
- **Error propagation:** Polling failures surface through React Query's existing error state. Pages already render `<ErrorState>` with retry. Polling continues after errors (React Query default) — the error state shows but the next successful poll clears it. No new error propagation paths.
- **State lifecycle risks:** No new persistent state — toggle state is ephemeral `useState` per mount. Navigating away and back resets to defaults. No localStorage, no URL state for toggle.
- **API surface parity:** `useTraceLlmCalls` (in `llmCalls.ts`) and `useTraceToolCalls` (in `toolCalls.ts`) are covered in Unit 3. GitHub hooks (`useGitHubIssue`, `useGitHubPull`) are explicitly excluded from polling — their `staleTime` handles freshness.
- **Integration coverage:** The main integration concern is that `refetchInterval` flows correctly from `useLiveRefresh` through the page component to the API hooks. Unit tests for each page should verify the hook receives the expected interval.
- **Unchanged invariants:** Home page's always-on 15s polling is explicitly unchanged. `useTaskDescendants`' own status-gated polling is unchanged. `useAgents()` global caching (no `refetchInterval`) is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| SQLite contention from multiple polling hooks on detail pages | Detail pages use 5s interval with max ~5 hooks = 60 req/min — within acceptable bounds per WAL checkpoint cadence. Multiple open tabs multiply this, but that's an operator choice. |
| Data shifting on list pages during polling (rows move under cursor) | `isDefaultView` guard prevents polling on filtered/paginated views. Default view shifts are acceptable since the user opted in via toggle. |
| `placeholderData: keepPreviousData` may show stale data briefly after error | Acceptable tradeoff — prevents jarring loading flash. Error state still renders normally via the standard ternary. |

## Sources & References

- Related issue: #662
- Related milestone: #13 (Dashboard improvements)
- Stitch reference: screen `c5b6feddb5444f3d83a7f9b94e140bcd` (Unified Event Timeline Dashboard)
- Existing pattern: `dashboard/src/pages/Timeline.tsx`, `dashboard/src/api/timeline.ts`
- Existing pattern: `dashboard/src/pages/Home.tsx` (`HOME_REFETCH_INTERVAL`)
- Existing pattern: `dashboard/src/api/tasks.ts` (`useTaskDescendants` conditional polling)
- Design constraint: `docs/design/luminescent-core.md` §wrapper-component constraint
- Learning: `docs/solutions/best-practices/dashboard-landing-page-widget-composition-2026-05-06.md`
- Learning: `docs/solutions/database-issues/dashboard-stale-wal-snapshot-2026-04-27.md`
