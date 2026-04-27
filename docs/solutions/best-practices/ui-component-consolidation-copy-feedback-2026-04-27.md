---
title: "Consolidate hand-rolled UI patterns to canonical shared primitives"
date: 2026-04-27
category: best-practices
module: packages/ui
problem_type: best_practice
component: tooling
severity: low
applies_when:
  - Adding copy-to-clipboard or other interactive micro-affordances to dashboard pages
  - Noticing duplicated UI patterns across dashboard page files
  - Refining visual transitions in shared component library
tags:
  - copy-button
  - ui-primitives
  - component-consolidation
  - css-transitions
  - dashboard
---

# Consolidate hand-rolled UI patterns to canonical shared primitives

## Context

`<CopyButton />` in `@senara-solutions/ui` already provided clipboard feedback via icon swap (Copy -> Check), but the swap was abrupt (no CSS transition). Meanwhile, `dashboard/src/pages/TraceDetail.tsx` had a hand-rolled duplicate of the entire CopyButton implementation (~30 lines) rather than importing the canonical primitive.

This pattern — hand-rolled duplicates of shared components — creates maintenance drift: when the primitive improves, duplicates don't benefit. When bugs are fixed in the primitive, duplicates retain the bug.

## Guidance

1. **Grep for `navigator.clipboard` (or the relevant browser API) across `dashboard/src/` and `packages/ui/src/` before shipping.** Only the canonical primitive (`CopyButton.tsx`) should contain the raw API call. Any other hits indicate a missed migration.

2. **Use CSS opacity transitions for icon-swap feedback, not framer-motion.** A 150ms `transition-opacity` crossfade between two absolutely-positioned icons (one visible, one hidden) produces a smooth visual confirmation without adding a dependency. The absolute-overlay pattern avoids React reconciliation artifacts from `key={state}` remounts under rapid clicks.

3. **Add `data-testid` attributes to shared primitives.** Querying by `data-testid` rather than SVG presence or button role makes tests stable across icon library upgrades (e.g., lucide-react swapping SVG internals).

## Why This Matters

Hand-rolled duplicates of shared components accumulate silently. A quick `grep -rn "navigator.clipboard" dashboard/src/` catches copy-button drift; similar greps work for other patterns (status pills, filter dropdowns, loading states). The `packages/ui/CLAUDE.md` enforcement rules exist precisely to prevent this — this learning reinforces the existing discipline with a concrete migration example.

## When to Apply

- Before any PR that adds interactive UI affordances to dashboard pages
- During milestone audits (e.g., mika#13 dashboard improvements) to catch accumulated drift
- When improving a shared primitive's visual behavior — check that all consumers use the import, not a local copy

## Examples

**Before (hand-rolled in TraceDetail.tsx):**
```tsx
// 30 lines duplicating CopyButton's exact logic
function CopyButton({ text, className, title }) {
  const [copied, setCopied] = useState(false)
  // ... navigator.clipboard.writeText, setTimeout, icon swap ...
}
```

**After (import the canonical primitive):**
```tsx
import { CopyButton } from '@senara-solutions/ui'
// Used directly: <CopyButton text={row.input} title="Copy input" />
```

**Smooth transition pattern (in the shared primitive):**
```tsx
<span className="relative inline-flex items-center justify-center w-3 h-3">
  <Copy className={`transition-opacity duration-150 ${copied ? 'opacity-0' : 'opacity-100'}`} />
  <Check className={`absolute transition-opacity duration-150 text-emerald-400 ${copied ? 'opacity-100' : 'opacity-0'}`} />
</span>
```

## Related

- [mika#665](https://github.com/senara-solutions/mika/issues/665) — Dashboard copy feedback
- [mika#13](https://github.com/senara-solutions/mika/issues/13) — Dashboard improvements milestone
- `packages/ui/CLAUDE.md` — Enforcement rules for hand-rolled patterns
- `docs/design/luminescent-core.md` — Rulebook section 5, transition states
