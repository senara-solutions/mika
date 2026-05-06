---
title: "Dashboard detail page tabs: URL path segments over query params"
date: 2026-05-06
category: best-practices
module: dashboard
problem_type: best_practice
component: tooling
severity: low
applies_when:
  - Adding tabbed navigation to a dashboard detail page
  - Migrating existing detail page tabs from query params to path segments
  - Deciding between ?tab= query params and /:tab? path segments for tab state
tags:
  - dashboard
  - url-state
  - react-router
  - tabs
  - path-segments
  - session-detail
---

# Dashboard detail page tabs: URL path segments over query params

## Context

The dashboard `SessionDetail` page originally used `?tab=llm-calls` query params (via `useSearchParams`) to sync tab state with the URL. While functional, path segments (`/sessions/:id/llm-calls`) are more natural for primary navigation tabs — they're easier to read, share, and bookmark. Issue mika#676 requested the migration.

The existing `?tab=` pattern is documented in `docs/solutions/best-practices/dashboard-url-state-shareable-links-pattern-2026-05-06.md` and remains the canonical approach for `AgentDetail` tabs. The two patterns now coexist intentionally — SessionDetail uses path segments, AgentDetail uses query params.

## Guidance

Use React Router v7's optional param syntax (`:tab?` suffix) to add tab routing to detail pages:

**Route definition** (`App.tsx`):
```
sessions/:sessionId/:tab?
```

**Tab state from params** (`SessionDetail.tsx`):
- Read tab from `useParams` instead of `useSearchParams`
- Validate with a type guard that accepts `string | null | undefined` (useParams returns `undefined` for missing optional params)
- Default to the first tab when the param is absent or invalid
- Navigate with `useNavigate(path, { replace: true })` to avoid polluting browser history

**Type guard pattern** — widen to accept `undefined` so TypeScript narrowing works directly on the `useParams` return value without needing `?? null` coercion or `as` casts:

```typescript
function isSessionTab(value: string | null | undefined): value is SessionTab {
  return value != null && VALID_SESSION_TABS.includes(value as SessionTab)
}

// Clean call site — no cast needed
const activeTab: SessionTab = isSessionTab(tabParam) ? tabParam : 'messages'
```

**Default-tab omission** — keep URLs clean by omitting the path segment for the default tab:
```typescript
const path = tab === 'messages'
  ? `/sessions/${sessionId}`
  : `/sessions/${sessionId}/${tab}`
navigate(path, { replace: true })
```

## Why This Matters

- **Bookmarkable/shareable URLs**: `/sessions/abc/llm-calls` is more readable than `/sessions/abc?tab=llm-calls`
- **Browser history**: `replace: true` prevents tab clicks from polluting the back button
- **SPA fallback compatibility**: The Axum embedded dashboard already serves `index.html` for all unmatched paths — nested path segments work without backend changes
- **Type safety**: Widening the type guard to accept `undefined` eliminates unsafe casts at the call site

## When to Apply

- When adding tabs to a new dashboard detail page and the tabs represent primary navigation (distinct content views, not panel toggles)
- When migrating existing `?tab=` query param tabs to path segments
- Not applicable to filter/pagination state — those remain as query params via `useSearchParamsFilter`

## Examples

**Before** (query params):
```typescript
const [searchParams, setSearchParams] = useSearchParams()
const rawTab = searchParams.get('tab')
const activeTab = isSessionTab(rawTab) ? rawTab : 'messages'
```

**After** (path segments):
```typescript
const { sessionId, tab: tabParam } = useParams<{ sessionId: string; tab?: string }>()
const navigate = useNavigate()
const activeTab = isSessionTab(tabParam) ? tabParam : 'messages'
```

## Related

- `docs/solutions/best-practices/dashboard-url-state-shareable-links-pattern-2026-05-06.md` — the `?tab=` pattern (still canonical for AgentDetail)
- `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md` — confirms SPA fallback handles nested paths
- mika#676 — the issue that motivated this migration
- mika#664 — cross-cutting URL state management ticket
