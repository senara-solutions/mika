---
title: "feat(ui): enhance @senara-solutions/ui for console consumption"
type: feat
status: completed
date: 2026-04-02
issue: "#392"
---

# feat(ui): enhance @senara-solutions/ui for console consumption

## Overview

Add semantic color tokens to `theme.css` and make the `EmptyState` component more flexible so `mika-cloud` console can adopt `@senara-solutions/ui` without duplicating components and theme tokens.

## Acceptance Criteria

- [x] `theme.css` includes `--color-success: #10b981`, `--color-warning: #f59e0b`, `--color-error: #ef4444` in `@theme` block
- [x] `EmptyState` accepts optional `title?: string` prop (rendered as heading above message)
- [x] `EmptyState` accepts optional `icon?: React.ReactNode` prop (defaults to `Inbox` icon for backward compatibility)
- [x] `EmptyState` accepts optional `variant?: 'minimal' | 'card'` prop (default: `'minimal'` — current behavior)
- [x] `EmptyStateProps` type is exported from the barrel file (`src/index.ts`)
- [x] All 11 existing dashboard call sites render identically (no visual regression)
- [x] `packages/ui/package.json` version bumped to `0.2.0`
- [x] Package builds successfully: `npm run build -w packages/ui`
- [x] Dashboard builds successfully: `npm run build --prefix dashboard`

## Context

The `mika-cloud` console (`web/`) duplicates components and theme tokens that should come from the shared library. Two changes unblock adoption:

1. **Semantic color tokens** — the console already uses `--color-success`, `--color-warning`, `--color-error` in its own CSS. Moving these to the shared theme makes them available to all consumers.

2. **Flexible EmptyState** — the console needs a card-style variant with custom icons and titles. The current component only accepts `message?: string` and hardcodes the `Inbox` icon.

**No changes to StatusBadge** — the console's StatusBadge handles domain-specific provisioning states and is intentionally different.

## Implementation

### 1. `packages/ui/src/theme.css` — add semantic color tokens

Add to the `@theme` block:
```css
--color-success: #10b981;
--color-warning: #f59e0b;
--color-error: #ef4444;
```

These are standard Tailwind emerald-500, amber-500, red-500 — good contrast on dark backgrounds. Consumers use them as `text-success`, `bg-warning`, etc. via Tailwind v4 `@theme`.

Existing shared components (`TaskStatusBadge`, `StatusBadge`, `badges.ts`) keep their hardcoded Tailwind classes — migrating them to semantic tokens is out of scope for this PR.

### 2. `packages/ui/src/components/EmptyState.tsx` — extend props

```tsx
export interface EmptyStateProps {
  message?: string      // Body text (default: "No data found")
  title?: string        // Optional heading above message
  icon?: React.ReactNode // Custom icon (default: <Inbox size={32} />)
  variant?: 'minimal' | 'card' // Layout variant (default: 'minimal')
}
```

**Variant behavior:**
- **`'minimal'`** (default) — current layout: centered, `py-16`, icon + message. No border, no background. This is what all 20 dashboard call sites get today.
- **`'card'`** — wrapped in a card container: `bg-bg-card border border-white/[0.05] rounded-2xl p-8`. Icon + title + message centered inside. For customer-facing UX in the console.

**Layout when title + message are both present:**
```
[icon]
Title        ← text-sm font-medium text-heading
Body message ← text-sm text-muted/60 (current styling)
```

**Default icon:** When `icon` is omitted, the `Inbox` icon renders (backward compatible). Pass `icon={null}` to suppress the icon entirely.

### 3. `packages/ui/src/index.ts` — export type

Add `EmptyStateProps` as a named type export alongside the existing `EmptyState` default export re-export.

### 4. Version bump

Bump `packages/ui/package.json` from `0.1.0` to `0.2.0` (new features, pre-1.0).

## File Change List

| File | Change |
|------|--------|
| `packages/ui/src/theme.css` | Add 3 semantic color tokens |
| `packages/ui/src/components/EmptyState.tsx` | Add `title`, `icon`, `variant` props; card variant layout |
| `packages/ui/src/index.ts` | Export `EmptyStateProps` type |
| `packages/ui/package.json` | Version `0.1.0` → `0.2.0` |

## Verification

```bash
npm run build -w packages/ui   # types generate correctly
npm run build --prefix dashboard  # no consumer regressions
```

## Sources

- Issue: #392
- Downstream: senara-solutions/mika-cloud#77
- Learnings: `docs/solutions/architecture-patterns/extract-shared-ui-package.md` — barrel export checklist, version bump rule, `@theme` syntax requirement
- Learnings: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — never generate Tailwind classes dynamically
