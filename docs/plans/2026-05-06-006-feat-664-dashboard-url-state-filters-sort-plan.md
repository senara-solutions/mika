---
title: "feat: Dashboard URL state — close remaining gaps for shareable links"
type: feat
status: active
date: 2026-05-06
---

# feat: Dashboard URL state — close remaining gaps for shareable links

## Overview

Most dashboard list pages already reflect filter and pagination state in URL query params via `useSearchParamsFilter`. This plan closes the remaining gaps: Tasks page section-level pagination, Agents page search, and detail page tab state.

## Problem Frame

Filter/sort/pagination state that is not URL-reflected breaks three user expectations: (1) "send the link to this filtered view" requires a screenshot instead of a URL, (2) browser back/forward does not restore previous state, and (3) page reload loses context. The existing `useSearchParamsFilter` hook already solves this for most list pages, but several surfaces still use `useState` for state that should be URL-reflected.

## Requirements Trace

- R1. Every list page reads its filter + pagination state from URL query params on load
- R2. User interactions with filters update the URL without full page reload
- R3. Browser back/forward restores previous state correctly
- R4. Copy-link-and-share reproduces the exact view for the recipient

## Scope Boundaries

- Sort UI (sortable column headers, sort dropdown) is **not** in scope — no list page has sort today, and the issue's stitch reference ("URL-state pattern applies per page as each is redesigned") positions sort as a per-page concern for future redesigns
- Server-side ORDER BY support (Rust API changes) is **not** in scope for the same reason
- Sort URL-state helpers (`updateSort`, `getSort`) are **not** in scope — they would have zero consumers. When sort UI lands for a specific page, the hook extension is a one-unit change alongside the actual consumer

### Deferred to Separate Tasks

- Sort URL-state helpers + sortable column headers component in `packages/ui/`: separate PR when the first page redesign needs it
- Server-side `sort_by`/`sort_order` query params on Rust endpoints: separate PR per endpoint
- Detail page sub-pagination URL state (e.g., `SessionDetail` messages page, LLM calls page within detail): these are secondary navigation within a detail view and rarely shared as links

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/hooks/useSearchParamsFilter.ts` — central URL-state hook, wraps `react-router`'s `useSearchParams`. All list pages use it. `updateFilter(key, value)` auto-resets `page` on change.
- `dashboard/src/hooks/useLiveRefresh.ts` — `isDefaultView` gate suppresses polling when filters active or page > 1. Tasks page will extend this to cover section-level page params.
- `dashboard/src/pages/Sessions.tsx` — canonical list page pattern: reads all filters from `searchParams.get(...)`, builds typed filter object, passes to API hook.
- `dashboard/src/pages/Tasks.tsx` — four sections (`WorkItemsSection`, `TeamRunTasksSection`, `StandaloneCallbacksSection`, `ScheduledSection`) each with `useState(1)` for pagination.
- `dashboard/src/pages/Agents.tsx` — search uses `useState('')`, client-side filtering.
- `dashboard/src/pages/SessionDetail.tsx` — `activeTab` uses `useState<SessionTab>('messages')`.
- `dashboard/src/pages/AgentDetail.tsx` — `memoryTab` uses `useState<MemoryTab>('sections')`.

### Institutional Learnings

- `docs/solutions/655-filter-primitives-unification.md` — No generics on SelectFilter; all filter values are strings for URL serialization safety.
- `docs/solutions/best-practices/dashboard-time-range-filter-full-stack-pattern-2026-04-27.md` — Canonical 4-layer pattern (design spec, shared primitive, dashboard page, server endpoint) for filter params.
- `docs/solutions/663-pagination-audit-canonical-primitive-enforcement.md` — `page` param already URL-reflected on all standard list pages via `setPage()`. Tasks page sections are the exception.
- `docs/solutions/best-practices/dashboard-live-refresh-consistency-pattern-2026-05-06.md` — `isDefaultView` guard determines whether live-refresh is active. Tasks page must extend it to cover section-level page params.

## Key Technical Decisions

- **Extend `useSearchParamsFilter` page-clearing, don't create a new hook:** The existing hook is used by every list page. Extend `updateFilter`'s page-reset logic to clear section-prefixed page params in addition to `page`. Export section page param key constants so Tasks page and the hook share a single source of truth.
- **Tasks page sections use URL-prefixed param keys:** Each section gets a distinct prefix (`wi_page`, `trt_page`, `cb_page`, `sched_page`) to avoid collisions. This mirrors the section-specific filter identity (each section has its own `trigger_type`/`action_type`) and enables deep-linking to a specific section's page. Sections whose page param is absent in the URL default to page 1.
- **Agents page search stays client-side but becomes URL-reflected:** The Agents page loads all agents in one call (no pagination, no server-side filter). The search is purely client-side filtering. Moving the search term to `?search=...` in the URL enables shareability without needing server-side search.
- **Detail page tabs use `searchParams.get('tab')` with fallback to default:** Simple `?tab=llm-calls` in the URL. No new hook — just read/write via the existing `useSearchParamsFilter` or raw `useSearchParams`.
- **Page-clearing uses explicit list, not regex:** The set of page-related params is small and finite (`page`, `wi_page`, `trt_page`, `cb_page`, `sched_page`). A simple `forEach` delete is more legible than a regex pattern.

## Open Questions

### Resolved During Planning

- **Should sort helpers land now without sort UI?** No — zero consumers means speculative abstraction. When sort UI lands for a specific page, `updateSort`/`getSort` are a one-unit addition alongside the actual consumer.
- **Should Tasks section pagination use a single `page` param with section context?** No — multiple sections on one page each need independent pagination. Prefixed keys (`wi_page`, `trt_page`, etc.) are the simplest approach.
- **Should Agents search become server-side?** No — the Agents endpoint returns all agents (small dataset, no pagination). Client-side filtering is appropriate. URL-reflecting the search term is the only change needed.

### Deferred to Implementation

- Exact prefix strings for Tasks section page params — the plan suggests `wi_page`, `trt_page`, `cb_page`, `sched_page` but the implementer may choose clearer names if these feel cryptic.

## Implementation Units

- [x] **Unit 1: Extend `useSearchParamsFilter` page-clearing for section params**

**Goal:** Extend `updateFilter` to clear all page-related params (both `page` and section-prefixed variants) when a filter changes. Export section page param key constants. Add a `setSectionPage(key, page)` helper for section-level pagination.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `dashboard/src/hooks/useSearchParamsFilter.ts`
- Test: `dashboard/src/hooks/__tests__/useSearchParamsFilter.test.ts`

**Approach:**
- Define and export `ALL_PAGE_PARAMS = ['page', 'wi_page', 'trt_page', 'cb_page', 'sched_page'] as const`
- In `updateFilter`, replace `next.delete('page')` with `ALL_PAGE_PARAMS.forEach(k => next.delete(k))` — ensures filter changes on the Tasks page reset all section pages to 1
- Add `setSectionPage(key: string, page: number)` that sets a specific section page param (e.g., `setSectionPage('wi_page', 3)`)
- Keep existing `setPage(page)` unchanged for standard list pages

**Patterns to follow:**
- Existing `updateFilter` pattern: set value or delete if empty, then delete page params
- All param values are strings (per `docs/solutions/655-filter-primitives-unification.md`)

**Test scenarios:**
- Happy path: `updateFilter('agent_id', 'mika-dev')` clears both `page` and all section page params (`wi_page`, `trt_page`, etc.)
- Happy path: `setSectionPage('wi_page', 3)` sets `wi_page=3` in URL without clearing other params
- Edge case: calling `updateFilter` when no page params exist does not error
- Integration: existing list pages (Sessions, Timeline, etc.) continue working — `updateFilter` still clears `page`, and the additional section-param clearing is a no-op on pages that don't use them

**Verification:**
- All existing list pages continue working (no regression from expanded page-clearing)
- Section page param constants are exported and importable by Tasks page

- [x] **Unit 2: Tasks page — URL-reflect section-level pagination**

**Goal:** Replace `useState(1)` pagination in each Tasks section with URL-reflected params via `useSearchParamsFilter`.

**Requirements:** R1, R2, R3, R4

**Dependencies:** Unit 1 (section page param constants and `setSectionPage` helper)

**Files:**
- Modify: `dashboard/src/pages/Tasks.tsx`

**Approach:**
- Each section reads its page from `searchParams.get('<prefix>_page')` and writes via `setSectionPage('<prefix>_page', n)` from `useSearchParamsFilter`
- Replace `const [page, setPage] = useState(1)` in `WorkItemsSection` with `Number(searchParams.get('wi_page')) || 1` and `setSectionPage('wi_page', n)`
- Same for `TeamRunTasksSection` (`trt_page`), `StandaloneCallbacksSection` (`cb_page`), `ScheduledSection` (`sched_page`)
- Update the page-level `isDefaultView` computation to include all section pages: `(wi_page ?? 1) === 1 && (trt_page ?? 1) === 1 && ...`
- The `Pagination` component's `onPageChange` calls the URL-based setter instead of `useState` setter

**Patterns to follow:**
- `Sessions.tsx` pattern: read `Number(searchParams.get('page')) || 1`, write via `setPage`
- TimeRangeFilter integration already works in Tasks — extend the same `useSearchParamsFilter` usage

**Test scenarios:**
- Happy path: loading `/tasks?wi_page=3` renders WorkItems section on page 3
- Happy path: clicking page 2 in TeamRunTasks updates URL to include `trt_page=2`
- Edge case: loading `/tasks` with no page params defaults all sections to page 1
- Edge case: changing the time range filter resets all section page params to 1
- Integration: browser back after paginating restores the previous section page
- Integration: `isDefaultView` returns false when any section page > 1 (live-refresh suppressed)

**Verification:**
- All four Tasks sections paginate via URL params
- Sharing a Tasks URL with `?wi_page=2&sched_page=3` reproduces the exact view
- Browser back/forward navigates section pagination correctly

- [x] **Unit 3: Agents page — URL-reflect search**

**Goal:** Move the Agents page search from `useState` to URL params so the search term is shareable and survives reload.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `dashboard/src/pages/Agents.tsx`

**Approach:**
- Import `useSearchParamsFilter`
- Read search from `searchParams.get('search') ?? ''`
- Keep a local `useState` for the input value (debounce / type-ahead) but sync committed value to URL via `updateFilter('search', value)`
- On load, initialize local state from URL param
- The "Clear search" action in `EmptyState` clears both local state and URL param

**Patterns to follow:**
- `Timeline.tsx` trace search pattern: local `useState` for typing, committed to URL on Enter/blur
- `Sessions.tsx` session search pattern: same local + URL split

**Test scenarios:**
- Happy path: loading `/agents?search=dev` shows the search input populated with "dev" and filters the agent list
- Happy path: typing "qa" and pressing Enter updates URL to `?search=qa`
- Edge case: loading `/agents` with no search param shows all agents
- Edge case: clearing search removes the `search` param from URL
- Integration: browser back after searching restores the previous search term and filtered view

**Verification:**
- Agents page search term is in the URL
- Sharing `/agents?search=mika-dev` shows the filtered view for the recipient
- Reload preserves the search

- [x] **Unit 4: Detail page tabs — URL-reflect active tab**

**Goal:** Move `SessionDetail.activeTab` and `AgentDetail.memoryTab` from `useState` to URL `?tab=...` params so tab state is shareable and survives back/forward.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `dashboard/src/pages/SessionDetail.tsx`
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Use `useSearchParams()` (or `useSearchParamsFilter`) to read `tab` from URL
- `SessionDetail`: default tab is `'messages'`; valid values: `'messages'`, `'llm-calls'`, `'tool-calls'`, `'skills'`, `'team-workspace'` (kebab-case in URL for readability)
- `AgentDetail`: default tab is `'sections'`; valid values: `'sections'`, `'facts'`, `'history'`
- Tab change updates `?tab=...` in URL via `setSearchParams`
- Invalid `tab` values in URL fall back to the default tab silently
- Sub-pagination within tabs (e.g., `llmCallsPage` in SessionDetail) stays as `useState` — these are secondary navigation within a tab and rarely shared

**Patterns to follow:**
- Same `searchParams.get('tab') ?? 'messages'` read pattern used across all filter pages

**Test scenarios:**
- Happy path: loading `/sessions/abc?tab=llm-calls` opens the LLM Calls tab
- Happy path: clicking the "Skills" tab updates URL to `?tab=skills`
- Edge case: loading `/sessions/abc` with no tab param defaults to "messages"
- Edge case: loading `/sessions/abc?tab=invalid` falls back to "messages"
- Happy path: loading `/agents/mika-dev?tab=facts` opens the Facts tab
- Integration: browser back after switching tabs restores the previous tab

**Verification:**
- Sharing a detail URL with `?tab=...` opens the correct tab for the recipient
- Browser back/forward between tabs works correctly

## Acceptance Verification Checklist

After completing Units 1-4, verify:
- [ ] Grep for remaining `useState` calls in `dashboard/src/pages/` that manage filter/pagination/tab state not backed by URL params — none should remain
- [ ] `/tasks?wi_page=3&sched_page=2` loads with the correct section pages
- [ ] `/agents?search=mika-dev` loads with the search populated and list filtered
- [ ] `/sessions/abc?tab=llm-calls` opens the LLM Calls tab
- [ ] `/agents/mika-dev?tab=facts` opens the Facts tab
- [ ] Browser back/forward across filter and pagination changes restores state
- [ ] Page reload preserves all URL-reflected state

## System-Wide Impact

- **Interaction graph:** `useSearchParamsFilter` is consumed by every list page. Changes to its page-clearing behavior (Unit 1) affect all consumers. The change is additive (clearing more params, not fewer), so existing behavior is preserved.
- **Error propagation:** No new error paths. Invalid URL params are handled by defaulting to safe values (page 1, default tab).
- **State lifecycle risks:** Multiple components on the Tasks page will share URL state. The section-prefix approach (`wi_page`, `trt_page`) eliminates collision risk. React Router's `setSearchParams` is atomic per call.
- **API surface parity:** No backend changes. All URL params are consumed client-side to build API request params that already exist.
- **Integration coverage:** Browser history integration (back/forward) is the key cross-layer concern — unit tests with `MemoryRouter` can verify URL reads, but manual testing is needed for real browser history behavior.
- **Unchanged invariants:** `useLiveRefresh` `isDefaultView` semantics are preserved — Tasks page extends it to cover section-page checks, but the gate behavior (suppress polling when non-default) is unchanged. Other pages' `isDefaultView` computations are untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Tasks section page params add URL clutter | Short prefixes (`wi_page`, `trt_page`, `cb_page`, `sched_page`). Params only appear when user paginates past page 1. |
| Browser history stack pollution from rapid filter changes | `setSearchParams` uses `replace: false` by default (pushes history entries). This matches user expectation — each filter change is a distinct state. If users complain about too many history entries, a future optimization can batch rapid changes. |
| Agents page search debounce vs URL sync | Local `useState` for typing, commit to URL on Enter/blur. Prevents URL thrashing on every keystroke. Same pattern as Timeline trace search. |

## Sources & References

- Related issues: #664, #659, #655, #663
- Related code: `dashboard/src/hooks/useSearchParamsFilter.ts`, `dashboard/src/pages/Tasks.tsx`, `dashboard/src/pages/Agents.tsx`, `dashboard/src/pages/SessionDetail.tsx`, `dashboard/src/pages/AgentDetail.tsx`
- Learnings: `docs/solutions/655-filter-primitives-unification.md`, `docs/solutions/best-practices/dashboard-time-range-filter-full-stack-pattern-2026-04-27.md`, `docs/solutions/663-pagination-audit-canonical-primitive-enforcement.md`
