---
title: "Migrate hand-rolled status pills to shared StatusBadge component"
date: 2026-04-27
category: best-practices
module: packages/ui
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding or modifying status indicators in dashboard pages
  - Creating new status pill patterns for success/error/warning states
  - Extending the StatusBadge variant set for new status domains
tags:
  - status-badge
  - design-system
  - dashboard
  - hand-rolled-pills
  - luminescent-core
  - packages-ui
  - tailwind-tokens
---

# Migrate hand-rolled status pills to shared StatusBadge component

## Context

Dashboard pages independently rendered status indicators (success/error dots, status pills) using inline JSX with hardcoded Tailwind color utilities (`bg-emerald-400`, `text-red-400`). Each page's implementation drifted: different gap values (`gap-1` vs `gap-1.5`), different text sizes (`text-[10px]` vs `text-sm` vs inherited), presence or absence of pill backgrounds, and inconsistent label casing. The luminescent-core rulebook (the design system contract) was silent on multi-state grammar, leaving each developer to improvise.

The pattern appeared across 9 dashboard pages with 4+ different visual treatments for the same data type (LLM call status, tool call success/fail, session state).

## Guidance

### 1. Generalize the shared component with a typed variant API

Replace binary or ad-hoc component APIs with a constrained variant union:

```tsx
// Before: binary prop, hardcoded colors
interface StatusBadgeProps { active: boolean }

// After: typed variant union, design-token-derived styles
type StatusBadgeVariant = 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'blocked'
interface StatusBadgeProps {
  variant: StatusBadgeVariant
  label: string
  dotPulse?: boolean
}
```

Map variants to design tokens (not Tailwind color utilities):

```tsx
const VARIANT_STYLES: Record<StatusBadgeVariant, { bg: string; text: string; dot: string }> = {
  success: { bg: 'bg-success/10', text: 'text-success', dot: 'bg-success' },
  warning: { bg: 'bg-warning/10', text: 'text-warning', dot: 'bg-warning' },
  // ...
}
```

### 2. Use typed adapter components for domain-specific vocabulary

Don't force every consumer to map domain status to variant. Create thin adapters:

```tsx
// TaskStatusBadge: typed task status -> StatusBadge variant
const TASK_VARIANT_MAP: Record<string, { variant: StatusBadgeVariant; label?: string }> = {
  pending: { variant: 'warning', label: 'PENDING' },
  completed: { variant: 'success', label: 'COMPLETED' },
  // ...
}
export default function TaskStatusBadge({ status }: { status: string }) {
  const mapped = TASK_VARIANT_MAP[status] ?? DEFAULT
  return <StatusBadge variant={mapped.variant} label={mapped.label ?? status.toUpperCase()} />
}
```

### 3. Write all Tailwind class names literally in source

Never derive class names via string interpolation. Tailwind's JIT compiler scans source for literal class names — dynamically constructed classes are purged in production builds. Use explicit mapping records, not `"bg-" + color`.

### 4. Extend the design system rulebook before the code

When the design system is silent on a pattern you need (e.g., multi-state grammar), extend the rulebook first (luminescent-core.md), then implement against it. This prevents future drift where the code introduces variants the rulebook doesn't declare.

### 5. Audit thoroughly — grep for the pattern, not the component name

Hand-rolled pills won't appear in import searches. Use structural greps:

```bash
# Find hand-rolled pills by their shape, not their name
grep -rn "inline-flex items-center.*rounded-full" dashboard/src/pages/
# Find hardcoded status colors bypassing design tokens
grep -rn "bg-emerald-400\|bg-red-400\|bg-amber-400" packages/ui/src/components/
```

In this migration, a TraceDetail.tsx file with two hand-rolled patterns was initially missed because the audit focused on the plan's named files. The structural grep caught it during review.

## Why This Matters

- **Visual consistency:** Same status data renders identically across all pages without per-page tuning
- **Maintenance cost:** Color changes, spacing adjustments, or accessibility improvements happen in one component, not N pages
- **Tailwind purge safety:** Literal class strings in a Record are scanner-visible; ad-hoc inline classes risk production purge
- **Semantic alignment:** Design token names (`--color-success`, `--color-blocked`) carry meaning; Tailwind utilities (`bg-emerald-400`) don't communicate intent

## When to Apply

- When creating any new dashboard page that shows status indicators
- When a design system extension adds new status semantics (e.g., a seventh variant)
- When migrating other hand-rolled patterns (source badges, channel pills) to shared components
- When the `packages/ui` enforcement rules in `packages/ui/CLAUDE.md` flag a review fail

## Examples

**Before (LlmCalls.tsx):**
```tsx
function statusBadge(status: string) {
  switch (status) {
    case 'success':
      return (
        <span className="inline-flex items-center gap-1 text-emerald-400">
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
          <span className="text-[10px]">success</span>
        </span>
      )
    // ...10 more lines per case
  }
}
// Usage: {statusBadge(row.status)}
```

**After (LlmCalls.tsx):**
```tsx
import { StatusBadge } from '@senara-solutions/ui'
import type { StatusBadgeVariant } from '@senara-solutions/ui'

function llmStatusVariant(status: string): { variant: StatusBadgeVariant; label: string } {
  switch (status) {
    case 'success': return { variant: 'success', label: 'Success' }
    case 'error': return { variant: 'error', label: 'Error' }
    default: return { variant: 'neutral', label: status }
  }
}
// Usage: <StatusBadge {...llmStatusVariant(row.status)} />
```

**Net effect:** ~25 lines of inline JSX per page replaced by ~5 lines of typed mapping + 1-line component call. Across 9 pages: ~190 lines deleted, ~50 lines added.

## Related

- `docs/solutions/architecture-patterns/extract-shared-ui-package.md` — packages/ui architecture and checklist
- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — Tailwind JIT purge failure mode
- `docs/solutions/architecture-patterns/enhance-shared-ui-emptystate-theme-tokens.md` — semantic token conventions
- `docs/design/luminescent-core.md` §5.1 — multi-state status grammar declaration
- `packages/ui/CLAUDE.md` — enforcement rules and canonical primitives table
- mika#657 — parent issue (visual rhythm: tokens + status pills)
- mika#663 — sibling issue (Pagination audit, shares packages/ui/CLAUDE.md)
