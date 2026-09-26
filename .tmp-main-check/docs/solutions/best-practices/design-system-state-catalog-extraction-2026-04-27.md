---
title: "Design system state catalog: extracting canonical loading, error, and empty state primitives"
date: 2026-04-27
category: best-practices
module: dashboard
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding new list or detail pages to the dashboard
  - Migrating hand-rolled loading/error/empty states to canonical primitives
  - Extending the @senara-solutions/ui component library with lifecycle state components
tags:
  - design-system
  - loading-state
  - error-state
  - empty-state
  - react-query
  - dashboard
  - ui-components
  - accessibility
---

# Design system state catalog: extracting canonical loading, error, and empty state primitives

## Context

The Mika dashboard had 16+ pages each hand-rolling three lifecycle states (loading, empty, error) via raw `<div>` elements with inconsistent styling: `text-muted/60` for loading, `text-red-400` for errors (violating design token conventions), and no retry affordances on error states. mika#651 documented the most visible symptom: unstyled `Failed to load issue` / `Failed to load PR` banners on the Dev Runs detail page with no retry button and no detail link.

The underlying drift was universal — every `useQuery` consumer re-implemented the same `isLoading ? ... : error ? ... : empty ? ... : content` ternary pattern independently, with no shared components enforcing the design system's visual grammar.

## Guidance

### 1. Codify the grammar before writing components

Add a section to the design rulebook (`luminescent-core.md` §5.5) declaring the three primitives, their variant APIs, ARIA contracts, and the "hand-rolled is a review fail" enforcement rule. This establishes the visual spec as a rule before code is written, matching the precedent from prior extractions (§5.1 status pills, §5.2 list rows, §5.3 filter affordances).

### 2. Token-before-consumer

Add any new design tokens to `theme.css` before the components reference them. For example, `--color-surface-container-high` was verified absent during planning and added before `<LoadingState />` consumed it for skeleton rectangles. This prevents broken references and establishes the token as a first-class design system citizen.

### 3. Three primitives, direct consumption

```tsx
// @senara-solutions/ui exports
<LoadingState variant="list" | "detail" />
<EmptyState message="..." action?: { label, onClick } />
<ErrorState message="..." retry?: () => void detailsHref?: string variant?: "list" | "detail-section" />
```

No wrapper component (`<QueryStates />`) that reads query-library state. The primitives are backend-agnostic; consumers wire them to `useQuery`'s `{ isLoading, error, data, refetch }` shape directly.

### 4. formatApiError utility for human-shaped error messages

```tsx
import { formatApiError } from '@senara-solutions/ui'

// Canonical callsite pattern — never pass error.message directly
<ErrorState message={formatApiError(error)} retry={() => refetch()} />
```

Four cases: network error → connectivity message, server `detail` field → detail text, Error instance → error.message, unknown → generic fallback. This ensures uniform error grammar across all pages and prevents raw stack traces or developer-internal text from reaching users.

### 5. Canonical ternary pattern

Every `useQuery` consumer follows the same shape:

```tsx
const { data, isLoading, error, refetch } = useMyQuery(filters)

{isLoading ? (
  <LoadingState variant="list" />
) : error ? (
  <ErrorState message={formatApiError(error)} retry={() => refetch()} />
) : !data || data.data.length === 0 ? (
  <EmptyState
    message="No items match your filters"
    action={hasFilters ? { label: 'Clear filters', onClick: clearFilters } : undefined}
  />
) : (
  /* happy-path content */
)}
```

Detail pages use `variant="detail"` and early returns instead of ternaries. Sub-section errors use `variant="detail-section"` for compact inline display.

### 6. ARIA and accessibility

- `<LoadingState />`: `role="status"` + `aria-live="polite"` for screen reader announcement
- `<ErrorState />`: `role="alert"` for screen reader escalation
- Action buttons are keyboard-accessible by default; skeleton rectangles are not focusable
- Error/empty icons are decorative (`aria-hidden="true"`)

## Why This Matters

Without canonical primitives, each new page re-invents the loading/error/empty UI, producing visual drift, inconsistent error handling, and missing accessibility attributes. The enforcement rule ("hand-rolled is a review fail") prevents regression. The `formatApiError` utility ensures no page accidentally exposes raw error internals to users.

The migration reduced ~150 lines of duplicated code across 16 pages while adding retry affordances, consistent skeleton placeholders, and ARIA semantics that didn't exist before.

## When to Apply

- When adding any new dashboard page that uses `useQuery`
- When reviewing PRs that touch dashboard pages — verify the canonical ternary pattern is followed
- When extending `@senara-solutions/ui` with new lifecycle state variants (e.g., a future `detail-section` variant for `<LoadingState />`)

## Examples

**Before (hand-rolled, per-page):**
```tsx
{isLoading ? (
  <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
) : error ? (
  <div className="text-red-400 py-8 text-center text-sm">
    Error: {error instanceof Error ? error.message : 'Unknown error'}
  </div>
) : ...}
```

**After (canonical primitives):**
```tsx
{isLoading ? (
  <LoadingState variant="list" />
) : error ? (
  <ErrorState message={formatApiError(error)} retry={() => refetch()} />
) : ...}
```

**Review finding during implementation:** When a page has multiple independent queries (e.g., SessionDetail has `useSessionDetail` + `useSessionMessages`), the retry callback must call ALL relevant refetch functions, not just one. The pattern `retry={() => { refetchSession(); refetchMessages() }}` prevents silent no-op retries when only the secondary query failed.

## Related

- mika#658 — Dashboard > Empty / loading / error states: extract canonical patterns into @senara-solutions/ui
- mika#651 — Dev Runs page unstyled error banners (closed by this migration)
- mika#657 — StatusBadge extraction (§5.1 precedent)
- mika#654 — ListRow extraction (§5.2 precedent)
- mika#655 — Filter unification (§5.3 precedent)
- `docs/design/luminescent-core.md` §5.5 — State catalog grammar
- `docs/solutions/best-practices/design-system-listrow-extraction-2026-04-27.md` — Prior ListRow extraction pattern
- `docs/solutions/best-practices/design-system-status-pill-migration-2026-04-27.md` — Prior StatusBadge extraction pattern
