---
title: "feat: Dashboard landing/home overview page"
type: feat
status: active
date: 2026-05-06
issue: 666
---

# feat: Dashboard landing/home overview page

## Overview

Add a landing page at the dashboard index route (`/`) that shows operators a single-screen "state of the world" — active agents, in-flight work items, dev runs, team runs, cost summary, and recent activity. Currently the index route renders the Event Timeline; this change promotes a purpose-built overview and moves Timeline to `/timeline`.

## Problem Frame

Operators currently check multiple pages (Sessions, Tasks, LLM Calls, Dev Runs, Team Runs) to understand what's happening. A home view collapses the "check multiple pages" loop into one screen. The stitch reconciliation map (Screen #3 "Tasks & Orchestration Overview", `976190aa2b314effb5a51ab8c581180e`) is the canonical design reference for this page and explicitly identifies ticket #666 as the anchor for milestone #13 page redesigns.

## Requirements Trace

- R1. `/dashboard` renders a landing overview, not a blank redirect to a list page
- R2. Widgets use shared primitives (`<StatusBadge>`, `<ListRow>`, `<LoadingState>`, `<ErrorState>`, `<EmptyState>`, `<TokenBudgetBar>`) from `@senara-solutions/ui`
- R3. Clicking any widget item navigates to the corresponding full page with matching filters pre-applied
- R4. Page has a LIVE indicator and auto-refresh
- R5. Layout follows stitch Screen #3 stacked sections concept (Work Items / Team Runs / Dev Runs / Cost / Activity)
- R6. Each widget independently handles loading/error/empty states via the canonical ternary pattern

## Scope Boundaries

- No new backend API endpoints — v1 composes existing hooks on the frontend
- No user-customizable widget layout (reorderable/pinnable) — that's a future iteration
- No new shared UI components in `packages/ui/` — use existing primitives only
- No responsive sidebar collapse (pre-existing gap, out of scope)
- No "Open PRs awaiting review" or "Failing CI" widgets — these require GitHub API integration not yet available in the dashboard API layer

### Deferred to Separate Tasks

- Dedicated `GET /api/v1/overview` backend endpoint for aggregated data: separate optimization PR if N+1 frontend calls cause perceptible latency
- Customizable widget layout (pinned/reorderable): future iteration per issue description
- GitHub-sourced widgets (open PRs, CI status): requires `MIKA_GITHUB_TOKEN` plumbing into dashboard API
- "Milestone progress" widget: requires milestone-type task aggregation query not yet available

## Context & Research

### Relevant Code and Patterns

- **Router:** `dashboard/src/App.tsx` — `<Route index element={<Timeline />} />` is the current index route to replace
- **Sidebar:** `dashboard/src/components/Sidebar.tsx` — `navItems` array, first entry `{ to: '/', label: 'Event Timeline', icon: Activity }`
- **Page structure pattern:** Header (h2 title + subtitle + controls) → Filter bar → Canonical lifecycle ternary → Happy path content → Pagination
- **Card pattern:** `bg-bg-card border border-white/[0.05] rounded-2xl p-5 hover:border-accent/30 hover:shadow-[0_0_30px_rgba(124,106,247,0.06)]` with `Link` wrapper
- **Existing hooks to compose:** `useAgents()`, `useTasks()`, `useDevRuns()`, `useTeamRuns()`, `useCostTrend()`, `useTimeline()`
- **CostTrendChart:** `dashboard/src/components/CostTrendChart.tsx` — reusable but has header/controls; landing page needs a compact variant
- **Auto-refresh precedent:** Timeline page uses `refetchInterval: 5000` for default view; task descendants use conditional 15s

### Institutional Learnings

- **Canonical loading/error/empty primitives are mandatory** — `docs/solutions/best-practices/design-system-state-catalog-extraction-2026-04-27.md`. Hand-rolling is a review fail per `packages/ui/CLAUDE.md`
- **Multiple independent queries: retry callback must call ALL refetch functions** — same learning
- **recharts hex colors, not Tailwind classes** — `docs/solutions/660-dashboard-cost-trend-chart-time-series.md`
- **`dashboard_db` for cross-agent queries** — `docs/solutions/656-core-memory-actionable-dashboard-patterns.md`. Per-agent data requires `state.resolve_agent()`. The landing page uses cross-agent list endpoints, so this is not a concern
- **`useAgents()` returns unpaginated `Agent[]`** — globally cached on `['agents']` query key, shared across callsites via React Query dedup

### External References

- Stitch Screen #3: `976190aa2b314effb5a51ab8c581180e` — Tasks & Orchestration Overview
- Design rulebook: `docs/design/luminescent-core.md` — no-line rule, tonal shifting, `rounded-2xl` cards, uppercase tracking-wide labels
- Stitch map: `docs/design/dashboard-stitch-map.md` § Phase 3 sequence

## Key Technical Decisions

- **Landing page becomes the index route:** The new `Home` page replaces `<Timeline />` at the index route. Timeline moves to `/timeline`. This matches the stitch map's directive that #666 "anchors the rest of the milestone" and the north-star principle that the home view is the operator's entry point
- **Compose existing API hooks, no new endpoint:** `useAgents()`, `useTasks({ status: 'in_progress', per_page: 5 })`, `useDevRuns({ status: 'active', per_page: 5 })`, `useTeamRuns({ status: 'active', per_page: 5 })`, `useCostTrend({})`, `useTimeline({ per_page: 8 })` — avoids backend changes, all hooks already exist and are well-tested
- **Single shared refetchInterval of 15s:** Six hooks at 5s each = 72 requests/minute against single-threaded SQLite. 15s is a safe default (~24 requests/minute) that still feels responsive. Applied via a shared constant, not per-hook
- **Per-widget independent lifecycle:** Each section renders its own `LoadingState`/`ErrorState`/`EmptyState` independently. No page-level aggregated error state — this is consistent with how all other dashboard pages handle multi-query sections and is simpler to implement
- **Compact cost chart via existing CostTrendChart with reduced height:** Rather than creating a new sparkline component, render `<CostTrendChart>` with `defaultRange="last 24h"` and reduced container height. The component already handles loading/error/empty internally
- **Widget click-through filter mapping:** Each widget's "View all" link and row clicks navigate with explicit query params matching the target page's `useSearchParamsFilter` expectations

## Open Questions

### Resolved During Planning

- **Route placement:** Landing page at index (`/`), Timeline at `/timeline`. Resolved by stitch map priority and north-star entry-point principle
- **Refresh interval:** 15s shared across all landing page hooks. Resolved by SQLite contention analysis (6 hooks * 4 requests/minute = 24 req/min, acceptable)
- **Widget item count:** 5 items per widget section, with "View all N" link showing total count from `PaginatedResponse.total`. Resolved by balancing information density with screen real estate
- **Fresh-install empty state:** When `useAgents()` returns empty, show a single page-level `<EmptyState>` with a contextual message instead of six individual empty widgets. Resolved by UX clarity — six "no data" blocks is confusing for a new user

### Deferred to Implementation

- Exact Tailwind class combinations for the section header typography — will mirror existing page headers but may need minor adjustments for the stacked-section density
- Whether `CostTrendChart` height reduction requires prop changes or just a container CSS override — try container-first, add prop if needed
- Exact copy for each widget's empty-state message — will be finalized during implementation with contextually appropriate text

## Implementation Units

- [x] **Unit 1: Route restructure — move Timeline, add Home route**

  **Goal:** Move the Timeline page from the index route to `/timeline` and create a placeholder `Home` page at the index route. Update Sidebar navigation.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Create: `dashboard/src/pages/Home.tsx`
  - Modify: `dashboard/src/App.tsx`
  - Modify: `dashboard/src/components/Sidebar.tsx`
  - Test: `dashboard/src/pages/__tests__/Home.test.tsx`

  **Approach:**
  - In `App.tsx`: import `Home`, set `<Route index element={<Home />} />`, add `<Route path="timeline" element={<Timeline />} />`
  - In `Sidebar.tsx`: add `{ to: '/', label: 'Overview', icon: LayoutDashboard }` as the first nav item (using `LayoutDashboard` from lucide-react). Change Timeline entry from `to: '/'` to `to: '/timeline'`. Keep `end={item.to === '/'}` for exact matching on the overview link
  - `Home.tsx` starts as a minimal page with the header structure (title "Overview", subtitle, LIVE badge placeholder) and no data — just enough to verify routing works

  **Patterns to follow:**
  - Sidebar `navItems` array structure in `dashboard/src/components/Sidebar.tsx`
  - Page header pattern from `dashboard/src/pages/Timeline.tsx`

  **Test scenarios:**
  - Happy path: navigating to `/` renders the Home page, not Timeline
  - Happy path: navigating to `/timeline` renders the Timeline page
  - Happy path: Sidebar shows "Overview" as first item with active state when at `/`
  - Happy path: Sidebar "Event Timeline" link points to `/timeline`
  - Edge case: Sidebar exact matching — visiting `/timeline` does not highlight the Overview nav item

  **Verification:**
  - `npm run build --prefix dashboard` succeeds
  - Visiting `/` in dev server shows the Home page
  - Visiting `/timeline` shows the Timeline page
  - Sidebar highlights the correct nav item for each route

- [x] **Unit 2: Active Agents summary widget**

  **Goal:** Add the first data-driven section to the Home page — a summary of all agents with their status and last-seen time.

  **Requirements:** R2, R3, R6

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `dashboard/src/pages/Home.tsx`
  - Create: `dashboard/src/components/home/AgentsSummaryWidget.tsx`
  - Test: `dashboard/src/components/home/__tests__/AgentsSummaryWidget.test.tsx`

  **Approach:**
  - Create a `home/` directory under `components/` for landing-page-specific widget components
  - `AgentsSummaryWidget` consumes `useAgents()` (already returns `Agent[]` with `active`, `last_seen`, `message_count`)
  - Section card styled with `bg-bg-card border border-white/[0.05] rounded-2xl` per convention
  - Section header: uppercase tracking-wide label "AGENTS" with count badge, "View all" link to `/agents`
  - Each agent rendered as a `<ListRow variant="navigable">` clicking through to `/agents/{id}`
  - Agent status shown via `<StatusBadge variant={agent.active ? 'success' : 'neutral'}>` with `dotPulse` for active agents
  - `formatRelativeTime(agent.last_seen)` for the last-seen timestamp
  - Canonical lifecycle ternary for loading/error/empty using `@senara-solutions/ui` primitives

  **Patterns to follow:**
  - Card styling from `dashboard/src/pages/Agents.tsx` grid cards
  - `ListRow` usage conventions from `packages/ui/CLAUDE.md`
  - Lifecycle ternary from any existing list page

  **Test scenarios:**
  - Happy path: renders agent list with names, active status badges, and last-seen times
  - Happy path: clicking an agent row navigates to `/agents/{agentId}`
  - Happy path: "View all" link navigates to `/agents`
  - Edge case: agent with `last_seen: null` shows "Never" or similar fallback
  - Edge case: agent with `active: false` shows neutral (not success) StatusBadge without dotPulse
  - Error path: `useAgents()` fails — shows `<ErrorState>` with retry
  - Edge case: `useAgents()` returns empty array — shows `<EmptyState>` with contextual message

  **Verification:**
  - Widget renders agents from the API
  - Status badges use the correct `@senara-solutions/ui` StatusBadge variants
  - Navigation to agent detail and agents list works

- [x] **Unit 3: Work Items and Runs widgets**

  **Goal:** Add three stacked sections: Active Work Items (tasks), Dev Runs, and Team Runs — the core of the "what's happening now" view per stitch Screen #3.

  **Requirements:** R2, R3, R5, R6

  **Dependencies:** Unit 2 (establishes the widget component pattern)

  **Files:**
  - Create: `dashboard/src/components/home/WorkItemsWidget.tsx`
  - Create: `dashboard/src/components/home/DevRunsWidget.tsx`
  - Create: `dashboard/src/components/home/TeamRunsWidget.tsx`
  - Modify: `dashboard/src/pages/Home.tsx`
  - Test: `dashboard/src/components/home/__tests__/WorkItemsWidget.test.tsx`
  - Test: `dashboard/src/components/home/__tests__/DevRunsWidget.test.tsx`
  - Test: `dashboard/src/components/home/__tests__/TeamRunsWidget.test.tsx`

  **Approach:**
  - **WorkItemsWidget:** `useTasks({ status: 'in_progress', per_page: 5 })`. Each row: task label, agent_id, `<TaskStatusBadge>`, `formatRelativeTime(updated_at)`. Row click → `/tasks/{id}`. "View all" → `/tasks?status=in_progress`. Also show count from `PaginatedResponse.total` in the section header ("WORK ITEMS (12)")
  - **DevRunsWidget:** `useDevRuns({ per_page: 5 })`. Each row: label, branch (if available), `<StatusBadge>` mapped from dev run status, PR link if `pr_url` present. Row click → `/dev-runs/{id}`. "View all" → `/dev-runs`
  - **TeamRunsWidget:** `useTeamRuns({ per_page: 5 })`. Each row: team_name, goal (truncated), `<StatusBadge>` from status, iteration N/M. Row click → `/team-runs/{id}`. "View all" → `/team-runs`
  - All three follow the same section card pattern established in Unit 2: card container, uppercase label header with count + "View all" link, `<ListRow variant="navigable">` items, canonical lifecycle ternary
  - Status mapping for DevRuns: `in_progress` → `info`, `completed` → `success`, `failed` → `error`, `cancelled` → `neutral`, `blocked` → `blocked`
  - Status mapping for TeamRuns: `running` → `info` with `dotPulse`, `completed` → `success`, `failed` → `error`

  **Patterns to follow:**
  - Widget component pattern from Unit 2's `AgentsSummaryWidget`
  - `TaskStatusBadge` usage from existing Tasks page
  - DevRun/TeamRun status badge mapping from existing Dev Runs and Team Runs list pages

  **Test scenarios:**
  - Happy path: WorkItemsWidget renders in-progress tasks with labels, agent IDs, and status badges
  - Happy path: DevRunsWidget renders dev runs with branch names and PR links when available
  - Happy path: TeamRunsWidget renders team runs with goal text and iteration progress
  - Happy path: Row clicks navigate to the correct detail pages
  - Happy path: "View all" links navigate to list pages with appropriate filter params
  - Edge case: WorkItemsWidget with zero in-progress tasks shows `<EmptyState>` with "No active work items" message
  - Edge case: DevRun with `branch: null` and `pr_url: null` omits those fields gracefully
  - Edge case: TeamRun goal text longer than display area is truncated with ellipsis
  - Error path: each widget independently shows `<ErrorState>` when its hook fails — other widgets remain functional
  - Integration: section header shows count from `PaginatedResponse.total` (e.g., "WORK ITEMS (12)" even though only 5 are rendered)

  **Verification:**
  - All three widgets render data from their respective API hooks
  - Status badges use canonical `@senara-solutions/ui` components (no hand-rolled pills)
  - Click-through navigation works with correct URL params

- [x] **Unit 4: Cost summary and Recent Activity widgets**

  **Goal:** Add the cost trend chart (compact) and a recent activity feed to complete the landing page's widget set.

  **Requirements:** R2, R3, R5, R6

  **Dependencies:** Unit 3 (establishes full widget pattern; Home page layout is taking shape)

  **Files:**
  - Create: `dashboard/src/components/home/CostSummaryWidget.tsx`
  - Create: `dashboard/src/components/home/RecentActivityWidget.tsx`
  - Modify: `dashboard/src/pages/Home.tsx`
  - Test: `dashboard/src/components/home/__tests__/CostSummaryWidget.test.tsx`
  - Test: `dashboard/src/components/home/__tests__/RecentActivityWidget.test.tsx`

  **Approach:**
  - **CostSummaryWidget:** Renders the existing `<CostTrendChart>` in a compact container. Calls `useCostTrend({})` (server defaults to last 24h). Sets variant to `'total'` (no toggle), reduced chart height via container constraint. Section header: "COST (24H)" with "View all" → `/llm-calls`. Show the `hasEstimatedPricing` footnote. The `onVariantChange` prop receives a no-op since the landing page locks to total view
  - **RecentActivityWidget:** Calls `useTimeline({ per_page: 8 })`. Each row shows the event type badge (via `eventTypeBadge()` from `@senara-solutions/ui`), agent name (via `getAgentColor()`), content preview, and relative timestamp. Row click → appropriate detail page based on event_type (session → `/sessions/{id}`, task → `/tasks/{id}`, etc.). "View all" → `/timeline` (the relocated Timeline page)
  - Both follow the established section card + canonical lifecycle ternary pattern

  **Patterns to follow:**
  - `CostTrendChart` usage from `dashboard/src/pages/LlmCalls.tsx`
  - Timeline event rendering from `dashboard/src/pages/Timeline.tsx`
  - `eventTypeBadge()` and `getAgentColor()` utils from `@senara-solutions/ui`

  **Test scenarios:**
  - Happy path: CostSummaryWidget renders cost chart with last-24h data
  - Happy path: RecentActivityWidget renders recent timeline events with type badges and agent colors
  - Happy path: clicking "View all" on cost widget navigates to `/llm-calls`
  - Happy path: clicking "View all" on activity widget navigates to `/timeline`
  - Edge case: cost trend returns empty buckets — chart shows `<EmptyState>` "No cost data"
  - Edge case: timeline returns zero events — shows `<EmptyState>`
  - Edge case: CostTrendChart `hasEstimatedPricing` flag renders the footnote
  - Error path: cost API fails — shows `<ErrorState>` with retry while other widgets remain functional

  **Verification:**
  - Cost chart renders in compact form without variant toggle controls
  - Recent activity shows event type badges matching the relocated Timeline page
  - "View all" links target the correct pages

- [x] **Unit 5: LIVE badge and auto-refresh**

  **Goal:** Add a LIVE indicator to the page header and enable auto-refresh on all landing page queries at a shared 15s interval.

  **Requirements:** R4

  **Dependencies:** Unit 4 (all widgets exist and can be polled)

  **Files:**
  - Modify: `dashboard/src/pages/Home.tsx`
  - Modify: `dashboard/src/components/home/AgentsSummaryWidget.tsx`
  - Modify: `dashboard/src/components/home/WorkItemsWidget.tsx`
  - Modify: `dashboard/src/components/home/DevRunsWidget.tsx`
  - Modify: `dashboard/src/components/home/TeamRunsWidget.tsx`
  - Modify: `dashboard/src/components/home/CostSummaryWidget.tsx`
  - Modify: `dashboard/src/components/home/RecentActivityWidget.tsx`
  - Test: `dashboard/src/pages/__tests__/Home.test.tsx`

  **Approach:**
  - Define a shared constant `HOME_REFETCH_INTERVAL = 15_000` in `Home.tsx` and pass it to each widget as a prop
  - Each widget adds `refetchInterval: props.refetchInterval` to its `useQuery` options
  - LIVE badge in the page header: a `<StatusBadge variant="success" label="LIVE" dotPulse />` positioned to the right of the page title. Visible when auto-refresh is active
  - The LIVE badge is always visible on the home page (no toggle in v1 — matches the stitch map's "LIVE badge at the top" requirement)

  **Patterns to follow:**
  - Timeline's `refetchInterval: 5000` pattern in `dashboard/src/pages/Timeline.tsx`
  - `StatusBadge` with `dotPulse` from `@senara-solutions/ui`

  **Test scenarios:**
  - Happy path: LIVE badge renders with success variant and pulsing dot
  - Happy path: all widget hooks receive `refetchInterval` and poll at the shared interval
  - Edge case: refetch interval does not fire when the browser tab is not visible (React Query's default behavior — verify it's not overridden)

  **Verification:**
  - LIVE badge visible in the page header
  - Network tab shows periodic refetches at ~15s intervals across all widget hooks

- [x] **Unit 6: Fresh-install empty state and layout polish**

  **Goal:** Handle the fresh-install case (no agents provisioned) with a single cohesive empty state instead of six independent empty widgets. Polish the overall page layout — section spacing, responsive grid for side-by-side widgets where it makes sense.

  **Requirements:** R1, R2, R6

  **Dependencies:** Unit 5 (all widgets and refresh in place)

  **Files:**
  - Modify: `dashboard/src/pages/Home.tsx`
  - Test: `dashboard/src/pages/__tests__/Home.test.tsx`

  **Approach:**
  - `Home.tsx` calls `useAgents()` directly (in addition to passing it to `AgentsSummaryWidget`). If agents returns empty or loading, render a page-level state instead of all six widgets
  - Page-level empty state: `<EmptyState title="Welcome to Mika" message="No agents are provisioned yet. Start by creating an agent." icon={Bot} />` — centered, with the contextual guidance
  - Page-level loading state: `<LoadingState variant="list" rows={6} />` — single skeleton while the agents query resolves (prevents flash of six loading skeletons)
  - Layout polish: the six widget sections stack vertically in a single column. Cost Summary and Recent Activity can sit side by side in a 2-column grid on wider viewports (`grid grid-cols-1 lg:grid-cols-2 gap-4`) since they are the secondary signals. The three "active work" sections (Work Items, Dev Runs, Team Runs) remain full-width stacked for scanning density
  - Section spacing: `space-y-4` between sections, consistent with the stitch map's stacked sections concept

  **Patterns to follow:**
  - `EmptyState` with `action` prop for onboarding CTA (see `packages/ui/` EmptyState API)
  - Grid layout from `dashboard/src/pages/Agents.tsx` card grid

  **Test scenarios:**
  - Happy path: when agents exist, all six widgets render in the stacked layout
  - Happy path: on large viewports, cost and activity widgets appear side by side
  - Edge case: fresh install with zero agents — shows single page-level EmptyState, not six empty widgets
  - Edge case: agents query is loading — shows single page-level LoadingState
  - Edge case: agents query fails — shows page-level ErrorState with retry (prevents rendering widgets that will all fail)
  - Edge case: agents exist but all other queries return empty — each widget shows its own EmptyState (page-level gate only applies to agents)

  **Verification:**
  - Fresh-install case shows a single cohesive empty state
  - Normal case shows all widgets in the correct layout
  - Responsive grid adjusts on viewport width changes

## System-Wide Impact

- **Interaction graph:** No callbacks, middleware, or observers affected. The landing page is a pure read-only composition of existing API hooks. No new API endpoints means no server-side changes
- **Error propagation:** Each widget independently handles its own errors via `<ErrorState>`. The page-level gate (Unit 6) catches the "server unreachable" case by checking `useAgents()` first. No cross-widget error coupling
- **State lifecycle risks:** React Query's `staleTime: 5000` (global default) means widgets may briefly show stale data after navigation and back. This is acceptable and consistent with all other dashboard pages
- **API surface parity:** The index route change (`/` → Home, `/timeline` → Timeline) is a URL-level change. No internal links outside the dashboard reference the dashboard's index route, so no external consumers break. The Sidebar is the only internal link source and is updated in Unit 1
- **Unchanged invariants:** All existing API endpoints, hooks, and shared components are consumed as-is with no modifications to their interfaces. The `CostTrendChart` component is used with its existing props — no API changes

## Click-Through Filter Mapping

| Widget | Target route | Query params |
|--------|-------------|-------------|
| Agents | `/agents` | (none — full list) |
| Agent row | `/agents/{agentId}` | (none — detail) |
| Work Items "View all" | `/tasks?status=in_progress` | `status=in_progress` |
| Work Item row | `/tasks/{taskId}` | (none — detail) |
| Dev Runs "View all" | `/dev-runs` | (none — full list) |
| Dev Run row | `/dev-runs/{taskId}` | (none — detail) |
| Team Runs "View all" | `/team-runs` | (none — full list) |
| Team Run row | `/team-runs/{runId}` | (none — detail) |
| Cost "View all" | `/llm-calls` | (none — full list) |
| Recent Activity "View all" | `/timeline` | (none — full list) |
| Activity row | Detail page by event type | (none — detail) |

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Six parallel API calls on page load cause perceptible latency | React Query deduplication + staleTime means cached data is shown instantly on re-visits. Fresh loads fire in parallel (not waterfall). Monitor and consider a `GET /api/v1/overview` endpoint if latency exceeds 2s |
| 15s polling adds sustained load to SQLite | 15s is conservative (24 req/min vs Timeline's 72 req/min at 5s). React Query pauses polling when the tab is hidden. Monitor via server access logs |
| Moving Timeline from `/` to `/timeline` breaks bookmarks | Low risk — the dashboard is an internal tool with a small user base. The Sidebar nav is the primary navigation method. No external links reference the dashboard's internal routes |
| `CostTrendChart` may not look good at reduced height | Try container constraint first; the component's `ResponsiveContainer` adapts. If axes are unreadable, suppress them with a compact-mode prop in a follow-up |

## Documentation / Operational Notes

- Update `dashboard/CLAUDE.md` Pages section to list the new Home/Overview page and the Timeline route change
- No operational or deployment changes — this is a frontend-only change embedded in the SPA

## Sources & References

- **Stitch Screen #3:** `976190aa2b314effb5a51ab8c581180e` (Tasks & Orchestration Overview)
- **Design rulebook:** `docs/design/luminescent-core.md`
- **Stitch reconciliation map:** `docs/design/dashboard-stitch-map.md` § Phase 3, item 1
- **Related issues:** #666 (this), #13 (milestone), #657 (visual rhythm), #654 (ListRow), #655 (filters), #662 (live-refresh)
- **Institutional learnings:** `docs/solutions/best-practices/design-system-state-catalog-extraction-2026-04-27.md`, `docs/solutions/660-dashboard-cost-trend-chart-time-series.md`, `docs/solutions/663-pagination-audit-canonical-primitive-enforcement.md`
