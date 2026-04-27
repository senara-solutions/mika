# @senara-solutions/ui — Shared Component Library

Vite library mode, published to GitHub Packages. Peer deps: React 19, Tailwind CSS v4, lucide-react.

## Design System

All components implement the [luminescent-core](../../docs/design/luminescent-core.md) rulebook. Design tokens live in `src/theme.css`. Before adding or modifying a component, read:

- [`docs/design/north-star.md`](../../docs/design/north-star.md) — the WHY
- [`docs/design/luminescent-core.md`](../../docs/design/luminescent-core.md) — the rulebook (colors, typography, surfaces, components, do/don'ts)

## Canonical Primitives

| Component | Purpose | API | Migration status |
|---|---|---|---|
| `<StatusBadge>` | Multi-state status indicator (6 variants: success/warning/error/info/neutral/blocked) | `{ variant, label, dotPulse? }` | Audited clean (mika#657) |
| `<TaskStatusBadge>` | Task-domain status — thin adapter delegating to `<StatusBadge />` via typed task→variant mapping | `{ status: string }` | Audited clean (mika#657) |
| `<Pagination>` | Table/list pagination | `{ page, totalPages, total, onPageChange }` | Audited clean (mika#663) |
| `<EmptyState>` | Empty data placeholder | `{ message }` | — |
| `<CopyButton>` | Click-to-copy with visual confirm | `{ text, className?, title? }` | — |
| `<MarkdownContent>` | Render markdown content | `{ content }` | — |
| `<ListRow>` | All `<tr>` row rendering in list/table surfaces (static, navigable, expandable) | `{ variant, onClick?, isExpanded?, onToggle?, ariaLabel? }` | Audited clean (mika#654) |
| `<SelectFilter>` | All categorical filters in dashboard list pages (channel, event type, status, success, etc.) | `{ ariaLabel, value, onChange, options }` | Audited clean (mika#655) |
| `<AgentFilter>` | All agent-selection filters — thin adapter delegating to `<SelectFilter />` via consumer-injected `agents` prop | `{ agents, value, onChange, emptyLabel? }` | Audited clean (mika#655) |

## Enforcement Rules

- **Hand-rolled status pills are forbidden.** Any dashboard or consumer code rendering its own colored dot + text status indicator is a review fail. Use `<StatusBadge variant="..." label="..." />`. For task statuses, use `<TaskStatusBadge status={...} />`.
- **Design tokens over hardcoded colors.** Status colors must reference design tokens (`--color-success`, `--color-warning`, `--color-error`, `--color-accent`, `--color-muted`, `--color-blocked`), not Tailwind color utilities (`bg-emerald-400`, `text-red-400`, etc.).
- **Escape hatch:** If a surface genuinely needs a pill shape not covered by `<StatusBadge />` (e.g., channel pills, source badges), document the justification in the PR description and name the gap. Do not silently hand-roll.
- **Hand-rolled list rows are forbidden.** Any dashboard list page rendering `<tr>` with row-level `onClick` or inline hover styling outside `<ListRow />` is a review fail. Use `<ListRow variant="static|navigable|expandable" />`. See `luminescent-core.md` §5.2 for the affordance grammar.
- **Hand-rolled categorical filters are forbidden.** Any dashboard list page rendering `<select>` for categorical filtering (agent, channel, status, event type, etc.) outside `<SelectFilter />` or `<AgentFilter />` is a review fail. See `luminescent-core.md` §5.3 for the filter affordance grammar.

### `<AgentFilter />` callsite pattern

Consumer is responsible for fetching agents. `<AgentFilter />` does NOT call `useAgents()` — preserves layer separation; library cannot depend on dashboard's API layer.

```tsx
const { data: agents } = useAgents()  // query key: ['agents'] — verify cache shape if duplicating
return <AgentFilter agents={agents} value={filters.agent_id ?? ''} onChange={(v) => updateFilter('agent_id', v)} />
```

## Commands

- `npm run build --prefix packages/ui` — Build the library
- `npm run dev:dashboard` — Dev server (builds ui first, requires mika-server on :8080)
