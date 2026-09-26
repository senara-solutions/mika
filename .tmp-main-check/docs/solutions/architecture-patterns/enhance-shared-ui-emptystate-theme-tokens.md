---
title: "Enhance shared UI library: EmptyState flexibility and semantic color tokens"
category: architecture-patterns
date: 2026-04-02
tags: [ui, react, tailwind, design-tokens, component-library, packages-ui]
module: packages/ui
---

# Enhance shared UI library: EmptyState flexibility and semantic color tokens

## Problem

The `mika-cloud` console (`web/`) duplicated components and theme tokens from `@senara-solutions/ui` because the shared library lacked semantic color tokens (`--color-success`, `--color-warning`, `--color-error`) and the `EmptyState` component was too rigid — it only accepted `message?: string` with a hardcoded `Inbox` icon and no layout variants.

## Root Cause

The shared UI package was extracted with only the dashboard's immediate needs in mind. A second consumer (mika-cloud console) needed richer empty states (card variant, custom icons, titles) and semantic colors that the theme didn't provide.

## Solution

### 1. Semantic color tokens in `theme.css`

Added to the `@theme` block (Tailwind v4 syntax):

```css
--color-success: #10b981;
--color-warning: #f59e0b;
--color-error: #ef4444;
```

These are standard Tailwind emerald-500, amber-500, red-500. Consumers use them as `text-success`, `bg-warning`, etc.

### 2. Extended EmptyState props

```tsx
export interface EmptyStateProps {
  message?: string          // Body text (default: "No data found")
  title?: string            // Optional heading above message
  icon?: ReactNode          // Custom icon (default: Inbox; null = no icon)
  variant?: 'minimal' | 'card'  // Layout variant (default: 'minimal')
}
```

**Key design decisions:**

- **Icon three-state convention:** `undefined` = default Inbox icon, `null` = suppress icon, `ReactNode` = custom icon. Implemented with a single derived variable: `const resolvedIcon = icon === null ? null : (icon ?? <Inbox size={32} />)`. An earlier two-variable approach (`showIcon` + `iconElement`) was simplified during review.

- **Variant defaults:** `'minimal'` is the default, preserving the exact current layout (`py-16`, no border/background) for all 11 existing dashboard call sites. `'card'` adds `bg-bg-card border border-white/[0.05] rounded-2xl p-8`.

- **Type export:** `EmptyStateProps` is exported from the barrel file for consumer type-checking.

### 3. Version bump

`0.1.0` → `0.2.0` (minor bump for new features, pre-1.0).

## Prevention

- When extracting shared components, design props with flexibility for multiple consumers from the start — even if the first consumer only needs a subset.
- Follow the barrel export checklist from `docs/solutions/architecture-patterns/extract-shared-ui-package.md`: export types, bump version, run full build (not just watch mode).

## Related

- [Extract shared UI package](extract-shared-ui-package.md) — original extraction pattern
- Issue: #392
- Downstream: senara-solutions/mika-cloud#77
