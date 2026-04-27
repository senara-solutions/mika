---
module: dashboard
date: 2026-04-27
problem_type: best_practice
component: tooling
severity: medium
tags:
  - ui-components
  - design-system
  - accessibility
  - keyboard-navigation
  - shared-primitives
  - list-rows
  - react
applies_when:
  - Adding a new list or table page to the dashboard
  - Modifying row rendering in any list page
  - Porting dashboard patterns to mika-cloud or other consumers of @senara-solutions/ui
---

# Extract shared list row primitives to prevent affordance drift

## Context

The Mika Observability Dashboard had 7+ list pages (Timeline, Sessions, Tasks, LlmCalls, ToolCalls, DevRuns, TeamRuns), each hand-rolling its own `<tr>` rendering with inline hover classes, onClick handlers, and glyphs. Three distinct row patterns emerged organically: static rows (cell-level links only), navigable rows (arrow glyph linking to detail), and expandable rows (chevron toggling inline expansion). Without a shared primitive, each page diverged slightly: inconsistent hover affordances, no keyboard accessibility on any clickable row, no ARIA attributes, and different glyph conventions.

## Guidance

Factor row rendering into a single `<ListRow />` component in `@senara-solutions/ui` with three typed variants:

- **`static`**: Pure structural row with consistent hover. Not focusable, no keyboard interaction. Cell-level links navigate individually.
- **`navigable`**: Whole row is clickable. Auto-injects `->` glyph in a leading `<td>`. Adds `tabIndex={0}`, `role="link"`, Enter key handler. Consumer passes `onClick` (use `useNavigate` from react-router for SPA navigation).
- **`expandable`**: Whole row toggles inline expansion. Auto-injects chevron-right/chevron-down glyph. Adds `tabIndex={0}`, `role="button"`, `aria-expanded`, Enter/Space/Escape key handlers.

Key implementation details:

1. **Nested-element guard**: The component uses `isTargetRow()` which checks `target.closest('a, button, [role="button"], [role="link"]')` against the row ref. This prevents row activation when clicking nested `<Link>` elements — consumers don't need `e.stopPropagation()`.

2. **Column alignment**: When mixing expandable and static rows in the same table (e.g., Tasks where root rows expand but children are static), the static rows need an empty `<td className="w-8" />` spacer to align with the expandable rows' auto-injected chevron column. The table header also needs `<th className="w-8 px-2 py-3" />` for the glyph column.

3. **SPA navigation**: The navigable variant uses `onClick` (not a `to` prop) because `@senara-solutions/ui` doesn't depend on `react-router`. Consumers call `useNavigate()` and pass the navigation function. This avoids full page reloads.

4. **Rulebook-first**: The design system rulebook (`luminescent-core.md` section 5.2) declares the row-affordance grammar before the component implements it. This prevents future deviation — the grammar is the contract, the component is the implementation.

## Why This Matters

Without a shared row primitive:
- Every new list page re-derives row semantics differently
- Keyboard accessibility (Tab/Enter/Space/Escape) is universally absent on clickable rows
- ARIA attributes (role, aria-expanded) are never added
- Design drift compounds across pages since there's no single point of enforcement
- Downstream consumers (mika-cloud) inherit the drift when they consume `@senara-solutions/ui`

With the primitive:
- `grep -rn "<tr.*hover:bg-white" dashboard/src/pages/` catches any hand-rolled rows as a review fail
- Keyboard a11y and ARIA come free with the variant choice
- New list pages inherit consistent behavior by importing `<ListRow />`

## When to Apply

- **Always** when adding a new list or table page to the dashboard
- **Always** when the row needs click behavior (use `navigable` or `expandable`, never inline `onClick` on `<tr>`)
- **On migration** when porting dashboard patterns to mika-cloud

## Examples

Before (hand-rolled expandable row in ToolCalls):
```tsx
<tr
  onClick={() => toggleExpand(row.id)}
  className="hover:bg-white/[0.02] transition-colors cursor-pointer"
>
  <td className="px-2 py-3 text-muted/30">
    {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
  </td>
  {/* ... cells with onClick={(e) => e.stopPropagation()} on every Link */}
</tr>
```

After (using ListRow):
```tsx
<ListRow
  variant="expandable"
  isExpanded={isOpen}
  onToggle={() => toggleExpand(row.id)}
  ariaLabel={`Toggle details for tool call ${row.tool_name}`}
>
  {/* ... cells without stopPropagation — ListRow handles nested-element guard */}
</ListRow>
```
