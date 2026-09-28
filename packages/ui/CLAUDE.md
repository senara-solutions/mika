# @samidarko/ui — Shared Component Library

Vite library mode, published to npmjs.org as a public package (`access: public`, token-free anonymous install — mika#1386). Peer deps: React 19, Tailwind CSS v4, lucide-react.

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
| `<LoadingState>` | All loading states — skeleton placeholders for list and detail pages | `{ variant: 'list' \| 'detail', rows?, ariaLabel? }` | Audited clean (mika#658) |
| `<EmptyState>` | All zero-result states — extended with optional action affordance | `{ message?, title?, icon?, variant?, action?: { label, onClick } }` | Audited clean (mika#658) |
| `<ErrorState>` | All fetch-failure states — retry + details affordances, no raw stack traces | `{ message?, retry?, detailsHref?, variant?: 'list' \| 'detail-section' }` | Audited clean (mika#658) |
| `<CopyButton>` | Click-to-copy with visual confirm | `{ text, className?, title? }` | — |
| `<MarkdownContent>` | Render markdown content | `{ content }` | — |
| `<ListRow>` | All `<tr>` row rendering in list/table surfaces (static, navigable, expandable) | `{ variant, onClick?, isExpanded?, onToggle?, ariaLabel? }` | Audited clean (mika#654) |
| `<SelectFilter>` | All categorical filters in dashboard list pages (channel, event type, status, success, etc.) | `{ ariaLabel, value, onChange, options }` | Audited clean (mika#655) |
| `<AgentFilter>` | All agent-selection filters — thin adapter delegating to `<SelectFilter />` via consumer-injected `agents` prop | `{ agents, value, onChange, emptyLabel? }` | Audited clean (mika#655) |
| `<TimeRangeFilter>` | All time-range filtering on dashboard list surfaces (presets + custom picker, ISO 8601 emission, server-side enforcement) | `{ value: { from?, to? }, onChange: (range) => void }` | Audited clean (mika#659) |
| `<TokenBudgetBar>` | Token/resource budget progress bar with three-tier color thresholds (green <60%, amber 60-85%, red >85%) and ARIA meter semantics. Use for bounded token/resource usage. | `{ value, max, thresholds?: { warning, danger }, label?, showFraction? }` | New (mika#656) |
| `<CostMeter>` | Unbounded threshold-based cost display with three-tier colors (neutral/warning/critical) and ARIA status semantics. Use for USD cost surfaces; use `<TokenBudgetBar>` for bounded token usage. Two variants: `full` (label + value) and `chip` (compact inline). | `{ value, warningAt?, criticalAt?, variant?, label?, ariaLabel? }` | New (mika#667) |
| `<LiveRefreshToggle>` | Auto-refresh toggle with LIVE badge — canonical affordance for all dashboard live-refresh surfaces | `{ isLive, onToggle, disabled?, className? }` | New (mika#662) |
| `<Button>` | All call-to-action buttons and CTA links across dashboard and landing. Three rulebook §5 variants: `primary` (the §2 `.mika-cta-gradient` texture), `secondary` (ghost), `tertiary` (text-only). The union discriminates on the **rendered element**, not on the variant: `as="link"` makes `disabled` / `loading` / `type` / `onClick` non-compilable rather than merely ignored. | action: `{ onClick, variant?, size?, icon?, ariaLabel?, type?, disabled?, loading? }` · link: `{ as: 'link', href, external?, variant?, size?, icon?, ariaLabel? }` | New (mika#1801) |
| `<Spinner>` | Unified loader. Consumed by `<Button loading>`, exportable standalone. Sizes are **not** a rulebook scale — §6 is Motion and declares no spinner sizes (mika#1801 R4); the three values map to the button sizes that consume them. | `{ size?: 'xs' \| 'sm' \| 'md', className?, ariaLabel? }` | New (mika#1801) |
| `<SecretField>` | Masked text field with a reveal toggle. Field chrome is rulebook §5 "Input Fields", applied literally; the *secret* grammar (masking, the Show/Hide control, monospace, `spellCheck={false}`) has **no rulebook section** and comes from the one measured caller. Named for what was measured: the console's only masked field is an API key, and it holds no password at all — `PasswordInput` would have named a population of zero. `autoComplete` is a flat prop, not a union: there is no login-vs-signup axis to discriminate on. The caller owns error rendering, so `invalid` / `describedBy` are what keep that error reachable. | `{ label, value, onChange, placeholder?, autoComplete?, invalid?, describedBy?, className? }` | New (mika#2562) |
| `<AuthCard>` | The centred card of the authentication screens, including the full-height centring both measured callers carry. `logo` is a consumer-injected slot for the same reason `<AgentFilter>` takes `agents` — the library cannot depend on a consumer's components. The `<h1>` level is fixed: on both callers this card *is* the page. The "Powered by…" footer is **not** part of it (one caller only, outside the card, viewport-fixed). | `{ title, subtitle?, logo?, children, className? }` | New (mika#2562) |
| `<Stepper>` | Onboarding progress indicator. An `<ol>`, **not** a `<nav>`: the steps are driven by server state and none is clickable, so a navigation landmark would announce navigation that does not exist. `current` is a step **id**, never an index — the step list is tier-dependent, so an index means a different step per tier. A `current` naming no displayed step reads as "past the end", which is the terminal step the caller filters out of the display. | `{ steps: { id, label }[], current, ariaLabel?, className? }` | New (mika#2562) |

## Enforcement Rules

- **Hand-rolled status pills are forbidden.** Any dashboard or consumer code rendering its own colored dot + text status indicator is a review fail. Use `<StatusBadge variant="..." label="..." />`. For task statuses, use `<TaskStatusBadge status={...} />`.
- **Design tokens over hardcoded colors.** Status colors must reference design tokens (`--color-success`, `--color-warning`, `--color-error`, `--color-accent`, `--color-muted`, `--color-blocked`), not Tailwind color utilities (`bg-emerald-400`, `text-red-400`, etc.).
- **Escape hatch:** If a surface genuinely needs a pill shape not covered by `<StatusBadge />` (e.g., channel pills, source badges), document the justification in the PR description and name the gap. Do not silently hand-roll.
- **Hand-rolled list rows are forbidden.** Any dashboard list page rendering `<tr>` with row-level `onClick` or inline hover styling outside `<ListRow />` is a review fail. Use `<ListRow variant="static|navigable|expandable" />`. See `luminescent-core.md` §5.2 for the affordance grammar.
- **Hand-rolled categorical filters are forbidden.** Any dashboard list page rendering `<select>` for categorical filtering (agent, channel, status, event type, etc.) outside `<SelectFilter />` or `<AgentFilter />` is a review fail. See `luminescent-core.md` §5.3 for the filter affordance grammar.
- **Hand-rolled lifecycle states are forbidden.** Any dashboard page rendering raw `Loading...` text, `text-red-400` error divs, or inline loading/error ternaries outside `<LoadingState />`, `<ErrorState />`, and `<EmptyState />` is a review fail. Error messages must use `formatApiError(error)` — never raw `error.message`. See `luminescent-core.md` §5.5 for the state catalog grammar.
- **Hand-rolled time-range filters are forbidden.** Any dashboard list page rendering relative-time presets or `<input type="datetime-local">` for time filtering outside `<TimeRangeFilter />` is a review fail. See `luminescent-core.md` §5.4 for the time-range affordance grammar.
- **Hand-rolled auto-refresh toggles are forbidden.** Any dashboard page rendering its own toggle switch + LIVE badge for auto-refresh outside `<LiveRefreshToggle />` is a review fail. Use `<LiveRefreshToggle isLive={...} onToggle={...} />`.
- **Hand-rolled CTA fills are forbidden.** Any `<button>` or `<a>` under `dashboard/src` or `site/src` carrying its own CTA background (`bg-accent`, `bg-white`, a raw gradient, or any other filled treatment) outside `<Button />` is a review fail. Use `<Button variant="primary|secondary|tertiary" />`; for a link, `<Button as="link" href={...} />`. See `luminescent-core.md` §5 for the button grammar and §2 for the signature gradient. **This rule has a machine half**, unlike its siblings above: `scripts/check-cta-primitives.sh` (CI job `cta-primitives-lint`) refuses a new hand-rolled CTA on both surfaces, and `scripts/test-check-cta-primitives.sh` holds it against its own failure modes. Its allowlist, `scripts/cta-primitives-allowlist.txt`, ships **empty** and is pinned empty by that test: adding the first entry means deleting the assertion first, in the same diff. A stale entry — one matching no real violation — also fails the build, so an exemption cannot outlive its cause. **When the detector fires, the resolution is to migrate the callsite, never to allowlist it.**
- **The 34 non-CTA buttons of the dashboard are deliberately out of `<Button />`** — icon buttons, section toggles, text links. Rulebook §5 does not describe that grammar, and inventing variants for it is a §8 decision reserved to Vincent. The detector lets them pass by construction; that is a scoped absence, not a gap.
- **`<SecretField>`, `<AuthCard>` and `<Stepper>` carry no enforcement rule, and that absence is the decision (mika#2562).** Every rule above is scoped on a population that is actually present — "any **dashboard** page…", "under `dashboard/src` or `site/src`". These three have **zero** callsites in this repo: their only consumer is `mika-cloud`. A rule reading "hand-rolled Stepper is a review fail" would therefore be green every day without ever looking at anything, and its silence would read as conformity — nominally the mika#2205 class, *a guard you have not deployed reads exactly like a healthy fleet*. Transposing the CTA rule's machine half would make it worse, not better: `scripts/check-cta-primitives.sh`'s allowlist ships empty because its population is real, whereas a stepper scan would be empty **by construction**. The rule belongs in the repo that holds the surface, written on that surface, and is part of LC.3's adoption pass — not here.

### Rulebook gaps these three surface (operator-owned, §8)

`luminescent-core.md` describes the field chrome of `<SecretField>` (§5 "Input Fields") and nothing else of the three. There is no section for the secret grammar, none for a centred auth card, and no mention of a stepper anywhere. The implementations above are therefore derived from measured `mika-cloud` callsites plus the §6/§7 constraints that bind any surface — never from an invented section, because §8 reserves adding a pattern to Vincent, with a section naming its consuming surfaces (§8.3). Surfaced here as an operator follow-up, the same way LC.1 surfaced the §5.5/§2 error-hex contradiction rather than resolving it in the rulebook.

### `<AgentFilter />` callsite pattern

Consumer is responsible for fetching agents. `<AgentFilter />` does NOT call `useAgents()` — preserves layer separation; library cannot depend on dashboard's API layer.

```tsx
const { data: agents } = useAgents()  // query key: ['agents'] — verify cache shape if duplicating
return <AgentFilter agents={agents} value={filters.agent_id ?? ''} onChange={(v) => updateFilter('agent_id', v)} />
```

### Lifecycle state callsite pattern

Every `useQuery` consumer renders three states before the happy path. The canonical ternary:

```tsx
import { LoadingState, ErrorState, EmptyState, formatApiError } from '@samidarko/ui'

const { data, isLoading, error, refetch } = useMyQuery(filters)

{isLoading ? (
  <LoadingState variant="list" />
) : error ? (
  <ErrorState message={formatApiError(error)} retry={() => refetch()} />
) : !data || data.data.length === 0 ? (
  <EmptyState message="No items match your filters" action={hasFilters ? { label: 'Clear filters', onClick: clearFilters } : undefined} />
) : (
  /* happy-path content */
)}
```

Detail pages use `variant="detail"` for `<LoadingState />` and early returns instead of ternaries. Sub-section errors use `variant="detail-section"` for compact inline display.

### `<TimeRangeFilter />` callsite pattern

`<TimeRangeFilter />` is URL-state friendly — read `from`/`to` from `searchParams`, pass back via `updateFilter`:

```tsx
import { TimeRangeFilter } from '@samidarko/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'

const { searchParams, updateFilter } = useSearchParamsFilter()
const value = {
  from: searchParams.get('from') ?? undefined,
  to: searchParams.get('to') ?? undefined,
}

return (
  <TimeRangeFilter
    value={value}
    onChange={(range) => {
      updateFilter('from', range.from ?? '')
      updateFilter('to', range.to ?? '')
    }}
  />
)
```

The component emits ISO 8601 UTC strings; backends compare TEXT columns lexicographically (chronological-equivalent for ISO 8601). Filter typings declare `from?: string; to?: string` — never `number`.

## Accessibility Standards

Every primitive in this library is CI-gated for accessibility via `jest-axe` (axe-core). The following standards apply to all new and modified components:

- **axe assertion required.** Every primitive must have `expect(await axe(container)).toHaveNoViolations()` in its test file. CI enforces this.
- **Keyboard handlers on interactive elements.** Non-button interactive elements (e.g., clickable rows) must have `onKeyDown` for Enter/Space and `tabIndex={0}`.
- **aria-label on icon-only buttons.** Buttons without visible text must have `aria-label`. Decorative icons must have `aria-hidden="true"`.
- **Live regions for async state changes.** State changes triggered by user action (e.g., "Copied!") must use `role="status"` with `aria-live="polite"`.
- **Design tokens over hardcoded colors.** Use `text-success`, `text-error`, etc. — never `text-emerald-400`, `text-red-400`, or similar Tailwind color utilities.
- **Focus indicators.** All interactive elements must have visible `focus-visible:ring` or equivalent focus styles.

**Review-fail criteria for new primitives:** missing axe test, no keyboard handler on interactive element, hardcoded color, missing aria-label on icon-only button, missing focus indicator.

**Audit history:** See `docs/audits/2026-05-06-dashboard-a11y-audit.md` for the initial audit and finding catalog.

## Commands

- `npm run build --prefix packages/ui` — Build the library
- `npm test --prefix packages/ui` — Run tests (includes axe-core a11y assertions)
- `npm run dev:dashboard` — Dev server (builds ui first, requires mika-spirit on :8080)
