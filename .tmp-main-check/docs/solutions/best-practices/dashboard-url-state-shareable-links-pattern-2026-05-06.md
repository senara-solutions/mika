---
title: Dashboard URL state pattern for shareable links
date: 2026-05-06
category: best-practices
module: dashboard
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding new filter, pagination, or tab state to dashboard pages
  - Converting local useState to URL-reflected state for shareability
  - Building multi-section pages with independent pagination
tags:
  - dashboard
  - url-state
  - useSearchParamsFilter
  - shareable-links
  - pagination
  - filters
  - react-router
---

# Dashboard URL state pattern for shareable links

## Context

Dashboard pages used `useState` for filter, pagination, and tab state. This broke shareability (URLs didn't capture view state), browser back/forward (state lost on navigation), and page reload (context lost). The `useSearchParamsFilter` hook already handled most list pages, but Tasks section-level pagination, Agents search, and detail page tabs were local-only.

## Guidance

All user-visible view state that affects what data is shown must be URL-reflected via `useSearchParamsFilter` or raw `useSearchParams`. The central patterns:

**Standard list page filters/pagination:** Use `useSearchParamsFilter` directly. Read with `searchParams.get('key')`, write with `updateFilter('key', value)` or `setPage(n)`. Filter changes auto-reset all page params via `ALL_PAGE_PARAMS`.

**Multi-section pages (Tasks):** Each section gets a prefixed page param (`wi_page`, `trt_page`, `cb_page`, `sched_page`). Use `setSectionPage(key, page)` which accepts only `SectionPageKey` typed values. The `updateFilter` call clears all section page params in addition to `page`.

**Search with typing (Agents):** Keep local `useState` for responsive input, commit to URL on Enter via `updateFilter('search', value)`. Sync local state back from URL on browser back/forward using `useEffect`:

```tsx
const committedSearch = searchParams.get('search') ?? ''
const [search, setSearch] = useState(committedSearch)
useEffect(() => { setSearch(committedSearch) }, [committedSearch])
```

**Detail page tabs (SessionDetail, AgentDetail):** Read `?tab=` from URL with a type guard for validation. Omit the default tab from the URL to keep links clean. Reset sub-tab pagination on tab switch:

```tsx
const VALID_TABS: readonly TabType[] = ['messages', 'llm-calls', 'tool-calls', 'skills']
function isValidTab(v: string | null): v is TabType {
  return VALID_TABS.includes(v as TabType)
}
const activeTab = isValidTab(rawTab) ? rawTab : 'messages'
```

**`isDefaultView` guard:** When a page uses `useLiveRefresh`, the `isDefaultView` computation must account for all URL-reflected state that affects the view. Section page params must be checked alongside filters.

## Why This Matters

Without URL-reflected state: "send the link to this filtered view" = screenshot. Browser back/forward doesn't restore previous state. Reload loses context. With URL state, every view is a deep link — shareable, bookmarkable, and history-navigable.

## When to Apply

- Adding any new filter, sort, or pagination control to a dashboard page
- Converting a detail page tab or sub-navigation from local state
- Building a page with multiple independently-paginated sections
- Any state that changes what data the user sees should be in the URL

## Examples

**Before (Tasks section pagination with local state):**
```tsx
function WorkItemsSection({ timeRange, refetchInterval }) {
  const [page, setPage] = useState(1)  // Lost on reload, not shareable
  // ...
  <Pagination page={page} onPageChange={setPage} />
}
```

**After (URL-reflected section pagination):**
```tsx
function WorkItemsSection({ searchParams, setSectionPage }) {
  const page = Number(searchParams.get('wi_page')) || 1
  // ...
  <Pagination page={page} onPageChange={(p) => setSectionPage('wi_page', p)} />
}
```

## Related

- `docs/solutions/655-filter-primitives-unification.md` — SelectFilter/AgentFilter string-only API for URL serialization safety
- `docs/solutions/best-practices/dashboard-time-range-filter-full-stack-pattern-2026-04-27.md` — Canonical 4-layer pattern for filter params
- `docs/solutions/663-pagination-audit-canonical-primitive-enforcement.md` — Pagination primitive enforcement
- `docs/solutions/best-practices/dashboard-live-refresh-consistency-pattern-2026-05-06.md` — `isDefaultView` guard pattern
- mika#664
