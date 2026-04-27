---
title: Dashboard time-range filter — full-stack implementation pattern
date: 2026-04-27
category: best-practices
module: dashboard
problem_type: best_practice
component: frontend_stimulus
severity: medium
applies_when:
  - Adding time-based filtering to a new dashboard list page
  - Extending an existing list surface with from/to query params
  - Building observability UIs that need relative-time presets plus custom date pickers
tags:
  - dashboard
  - time-range
  - filter
  - iso-8601
  - packages-ui
  - axum
  - query-params
---

# Dashboard time-range filter — full-stack implementation pattern

## Context

The Mika observability dashboard displays sessions, tasks, dev-runs, LLM calls, tool calls, team runs, and timeline events. Users needed to filter these lists by time range to focus on recent activity or investigate a specific window. The implementation required changes across four layers: design system docs, shared UI library, dashboard pages, and server endpoints — coordinated to emit and consume ISO 8601 UTC strings consistently.

## Guidance

### 1. Design system specification first

Add the affordance grammar to `docs/design/luminescent-core.md` before writing code. This documents the interaction contract (presets, custom picker, clear semantics) and prevents ad-hoc divergence across pages.

### 2. Shared primitive in `packages/ui/`

Create a single `<TimeRangeFilter />` component in `@senara-solutions/ui` rather than duplicating filter logic per page. The component:

- Accepts `value: { from?: string; to?: string }` and `onChange: (range) => void`
- Ships default presets (15m, 1h, 24h, 7d, 30d) with override capability
- Emits ISO 8601 UTC strings via `new Date(localInput).toISOString()`
- Handles external clear (resets visual state when `value` becomes empty)

```tsx
import { TimeRangeFilter } from '@senara-solutions/ui'

<TimeRangeFilter
  value={{ from: searchParams.get('from') ?? undefined, to: searchParams.get('to') ?? undefined }}
  onChange={(range) => {
    updateFilter('from', range.from ?? '')
    updateFilter('to', range.to ?? '')
  }}
/>
```

### 3. Server-side enforcement via TEXT comparison

ISO 8601 strings sort lexicographically in chronological order. Server endpoints accept `from` and `to` as optional query parameters and filter with SQL `WHERE created_at >= ? AND created_at <= ?` on TEXT columns — no date parsing needed.

```rust
// In Axum query params struct:
pub from: Option<String>,  // ISO 8601
pub to: Option<String>,    // ISO 8601

// In SQL query builder:
if let Some(from) = &params.from {
    query = query + " AND created_at >= ?";
    bindings.push(from);
}
```

### 4. Enforcement via review rules

Add an enforcement rule to `packages/ui/CLAUDE.md` making hand-rolled time filters a review fail. This prevents regression to per-page implementations.

## Why This Matters

- **Consistency:** All 7 list pages use the identical filter with the same presets and behavior
- **Type safety:** ISO 8601 strings avoid the ambiguity of Unix timestamps (seconds vs milliseconds) and timezone confusion
- **Server simplicity:** TEXT lexicographic comparison avoids date parsing libraries on the Rust side
- **Maintainability:** Changing preset options or adding timezone support requires editing one component, not seven pages

## When to Apply

- Adding any new list/table page to the dashboard that displays timestamped records
- Adding time filtering to an existing page that currently lacks it
- Building any UI surface that needs relative-time presets (e.g., "last 24h") combined with custom date picking

## Examples

**Before (no shared primitive):**
```tsx
// Each page reimplements its own filter state, presets, and date inputs
const [since, setSince] = useState<number>(Date.now() - 86400000)
// ... duplicated across 7 pages with subtle differences
```

**After (shared primitive):**
```tsx
// Every page uses the same 3-line pattern
const value = { from: searchParams.get('from') ?? undefined, to: searchParams.get('to') ?? undefined }
<TimeRangeFilter value={value} onChange={(r) => { updateFilter('from', r.from ?? ''); updateFilter('to', r.to ?? '') }} />
```

**Server endpoint pattern:**
```rust
#[derive(Deserialize)]
pub struct ListParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub from: Option<String>,
    pub to: Option<String>,
    // ... other filters
}
```

## Related

- Design spec: `docs/design/luminescent-core.md` section 5.4
- Component source: `packages/ui/src/components/TimeRangeFilter.tsx`
- Enforcement rules: `packages/ui/CLAUDE.md`
- GitHub issue: senara-solutions/mika#659
- Related unification: `docs/solutions/best-practices/655-filter-primitives-unification.md`
