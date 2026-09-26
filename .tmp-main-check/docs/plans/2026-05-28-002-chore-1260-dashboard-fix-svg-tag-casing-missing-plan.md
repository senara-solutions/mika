# Plan: Fix SVG tag casing + missing list key warning in CostTrendChart

**Ticket:** mika issue#1260
**Type:** chore (code quality)
**Severity:** p3-nice-to-have

## Problem

Dashboard tests (`npm test -w dashboard`) pass green but emit React warnings from `CostTrendChart.tsx` and its test file. The warnings are:

1. **SVG tag casing warnings** — The recharts mock in `CostTrendChart.test.tsx` renders `AreaChart` as a `<div>`, but the component's `<defs>` block contains real SVG elements (`<linearGradient>`, `<stop>` with `stopColor`/`stopOpacity` attributes). When these SVG-namespaced elements render inside a `<div>` (HTML context, not SVG context), React warns about unrecognized DOM properties and incorrect element casing.

2. **Missing list key warning** — Likely originates from the ternary pattern inside `<defs>` (lines 261–273) or the conditional `<Area>` rendering (lines 294–316), where a ternary switches between a single element and a `.map()` array. React may also warn about fragments in the screen-reader `<table>` section.

## Root Cause

The recharts mock wraps children in `<div>` instead of `<svg>`:
```tsx
AreaChart: ({ children }) => <div data-testid="area-chart">{children}</div>
```

This causes the un-mocked `<defs>` block (with `<linearGradient>` and `<stop>` SVG elements) to render in an HTML context, triggering SVG casing warnings.

## Implementation

### Step 1 — Reproduce warnings

Run `npm test -w dashboard` and capture the specific warning text to confirm the exact elements and attributes React complains about.

**Files:** none (observation step)

### Step 2 — Fix the recharts mock SVG context

In `dashboard/src/components/CostTrendChart.test.tsx`, update the `AreaChart` mock to wrap children in `<svg>` instead of `<div>`:

```tsx
AreaChart: ({ children }: { children: React.ReactNode }) => (
  <svg data-testid="area-chart">{children}</svg>
),
```

This ensures SVG child elements render in a proper SVG namespace context, eliminating the casing warnings.

**Files:** `dashboard/src/components/CostTrendChart.test.tsx` (line 7)

### Step 3 — Fix missing key warning

Investigate the exact source of the missing key warning. Two likely locations in `CostTrendChart.tsx`:

**Option A — `<defs>` ternary (lines 261–273):** If the `.map()` branch of the ternary inside `<defs>` needs a wrapping fragment, wrap it:
```tsx
<defs>
  {variant === 'total' ? (
    <linearGradient id="costGradient" ...>...</linearGradient>
  ) : (
    agentIds.map((id) => (
      <linearGradient key={id} ...>...</linearGradient>
    ))
  )}
</defs>
```
Keys are already present on the `.map()` calls, so this may not be the issue.

**Option B — Screen-reader table `<th>` row (lines 342–351):** The `<th>` elements inside the total-variant `<>...</>` fragment don't need keys, but check whether React warns in the test environment.

**Option C — Mock artifact:** The missing key may only appear because the mock flattens the recharts component tree. If so, the SVG fix in Step 2 may resolve it. Verify after applying Step 2.

**Files:** `dashboard/src/components/CostTrendChart.tsx` (conditionally, only if warnings persist after Step 2)

### Step 4 — Verify zero warnings

Run `npm test -w dashboard` and confirm:
- All 91+ tests pass
- Zero React warnings emitted (no SVG casing, no missing key)
- Chart renders identically (no behavioral change — this is test-infrastructure and warning cleanup only)

**Files:** none (verification step)

## Files Changed

| File | Change |
|------|--------|
| `dashboard/src/components/CostTrendChart.test.tsx` | Update recharts mock `AreaChart` to use `<svg>` wrapper |
| `dashboard/src/components/CostTrendChart.tsx` | Fix missing key prop if applicable (conditional on Step 3 investigation) |

## Risk Assessment

**Low risk.** Changes are limited to:
- Test mock infrastructure (no production code path affected by SVG mock change)
- Potentially adding a `key` prop to a list element (zero behavioral impact)

No API changes, no rendering changes, no new dependencies.

## Acceptance Criteria (from ticket)

- [ ] Reproduce the warnings: `npm test -w dashboard` and observe warnings emitted
- [ ] Fix SVG tag casing in `CostTrendChart.tsx:260`
- [ ] Fix the missing `key` prop on the list-rendering pattern
- [ ] `npm test -w dashboard` passes green with zero React warnings emitted
- [ ] No behavior change — the chart renders identically before/after
