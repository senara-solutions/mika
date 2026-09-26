---
status: pending
priority: p2
issue_id: 689
tags: [code-review, architecture, quality]
dependencies: []
---

# Remove Unused AppShell/Sidebar Layout Components from UI Package

## Problem Statement

`packages/ui/src/layout/AppShell.tsx` and `packages/ui/src/layout/Sidebar.tsx` are exported from the UI package but **not imported by any consumer**. The dashboard continues to use its own `dashboard/src/components/Layout.tsx` and `dashboard/src/components/Sidebar.tsx`. These are premature abstractions with zero consumers — the generalized versions are both more complex and less functional than the originals (e.g., the extracted Sidebar requires a `renderLink` callback, losing react-router's `NavLink` active-state styling).

## Findings

- `AppShell` takes a `sidebar` prop instead of hardcoding `<Sidebar />` — generalization with zero consumers
- `Sidebar` adds `renderLink` callback, `NavItem` and `SidebarBrand` types — all for hypothetical future apps
- Dashboard's actual `Sidebar.tsx` uses `NavLink` from react-router with `isActive` className callback
- The `index.ts` barrel exports `AppShell`, `Sidebar`, `NavItem`, and `SidebarBrand` types that have no consumers

## Proposed Solutions

### Option A: Delete the unused layout components
- Remove `AppShell.tsx`, `Sidebar.tsx`, and their type exports from `index.ts`
- **Pros:** Removes dead code, follows YAGNI
- **Cons:** Need to re-create if/when a second consumer appears
- **Effort:** Small
- **Risk:** None

### Option B: Refactor dashboard to use the extracted components
- Update `dashboard/src/components/Layout.tsx` to compose `AppShell` + `Sidebar`
- Pass `NavLink` via `renderLink` prop
- **Pros:** Completes the extraction, validates the abstractions
- **Cons:** More work, forces routing abstraction on the dashboard
- **Effort:** Medium
- **Risk:** Low

## Recommended Action

*(To be filled during triage)*

## Technical Details

- **Affected files:**
  - `packages/ui/src/layout/AppShell.tsx` (17 lines, zero importers)
  - `packages/ui/src/layout/Sidebar.tsx` (48 lines, zero importers)
  - `packages/ui/src/index.ts` (type exports for NavItem, SidebarBrand)
  - `dashboard/src/components/Layout.tsx` (the actual layout in use)
  - `dashboard/src/components/Sidebar.tsx` (the actual sidebar in use)

## Acceptance Criteria

- [ ] Either unused components are deleted OR dashboard is refactored to consume them
- [ ] No dead exports in `packages/ui/src/index.ts`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |

## Resources

- [PR #193](https://github.com/senara-solutions/mika/pull/193)
