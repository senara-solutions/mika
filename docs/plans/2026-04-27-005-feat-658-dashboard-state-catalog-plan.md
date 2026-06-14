---
title: "feat(ui+dashboard): extract <LoadingState />, extend <EmptyState />, add <ErrorState />, migrate every list/detail page"
type: feat
status: active
date: 2026-04-27
origin: senara-solutions/mika#658
---

# Plan — state catalog (`<LoadingState />` + `<EmptyState />` + `<ErrorState />`) (mika#658)

**Issue:** [mika#658](https://github.com/senara-solutions/mika/issues/658) — `Dashboard > Empty / loading / error states: extract canonical patterns into @senara-solutions/ui`
**Branch:** `feat/658/dashboard-empty-loading-error-states`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** enhancement, dashboard
**Stitch reference (generated 2026-04-27):** screen `be408326efc949e49b8ab6d7c524b5f9` ("Mika State Catalog Reference") in project `6562713725762717689`. Canonical visual spec for the three primitives.

## Problem (per issue body, design now landed)

Every list and detail page has three non-happy states (loading, empty, error). Today each is hand-rolled per page — 18+ pages with repeated `isLoading ? <div>Loading...</div> : error ? <div>...</div> : ...` ternaries. mika#651 documented an unstyled `Failed to load issue` / `Failed to load PR` banner with no retry and no detail link as the most visible symptom; the underlying drift is universal.

The body's leading callout previously read "design-needed — requires Stitch session before dispatch (no canonical state catalog exists)." That blocker is now resolved: Stitch screen `be408326efc949e49b8ab6d7c524b5f9` provides the canonical visual spec for `<LoadingState />`, `<EmptyState />`, and `<ErrorState />` across both list and detail contexts. Issue body update will replace the design-needed callout with the screen reference.

## Audit results (verified during planning)

### Current `<EmptyState />` shape (partial; extend, don't replace)

**File:** `packages/ui/src/components/EmptyState.tsx`

```typescript
export interface EmptyStateProps {
  message?: string
  title?: string
  icon?: ReactNode
  variant?: 'minimal' | 'card'
}
```

API is forward-compatible with what the state catalog needs. **Plan extends, not replaces** — adding an optional `action?: { label: string; onClick: () => void }` prop for the "Clear filters" affordance shown in Stitch row-1 panel 2. No breaking change to existing 6+ callsites that already use `<EmptyState message="..." />`.

### Hand-rolled loading + error patterns (~18 pages drift)

Every paginated list page and detail page renders the same shape (verified via grep):

```tsx
{isLoading ? (
  <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
) : error ? (
  <div className="text-red-400 py-8 text-center text-sm">
    Error: {error instanceof Error ? error.message : 'Unknown error'}
  </div>
) : !data || data.data.length === 0 ? (
  <EmptyState message="No <thing> match your filters" />
) : (
  /* actual content */
)}
```

Variants observed:
- **List loading:** raw text `Loading...`, no skeleton, no spinner — visual void.
- **List error:** raw text `Error: <message>`, `text-red-400` class (not design-token-derived), no retry button, no fallback link.
- **List empty:** uses existing `<EmptyState />` cleanly (the one consistent pattern).
- **Detail page loading:** same `Loading...` shape applied to the whole page.
- **Detail page error:** same `Error:` shape — including mika#651's `Failed to load issue` / `Failed to load PR` callouts which are the visible UX problem driving this ticket.
- **Detail sub-section loading/error:** same shape repeated inline within sub-panels (e.g., "Tool Calls" sub-section on `LlmCallDetail`, "Recent Sessions" on `AgentDetail`).

Verified pages with the pattern (count in parentheses indicates separate `isLoading`/`error` ternary callsites per file, from earlier audit):
- `Tasks.tsx` (5), `DevRuns.tsx` (1), `Agents.tsx` (1), `Sessions.tsx` (1), `TeamRuns.tsx` (1), `LlmCalls.tsx` (1), `TeamRunDetail.tsx` (1), `TaskDetail.tsx` (1), `DevRunDetail.tsx` (1), `LlmCallDetail.tsx` (multiple sub-sections), `ToolCalls.tsx` (1), `ToolCallDetail.tsx` (sub-sections), `SessionDetail.tsx` (multiple sub-sections), `AgentDetail.tsx` (multiple sub-sections), `Timeline.tsx` (1), `Traces.tsx` (1), `TraceDetail.tsx` (sub-sections).

Total migration target: ~25-30 lifecycle ternary callsites across 17 files.

### TanStack Query usage (universal)

Every list/detail page uses `useQuery` from `@tanstack/react-query`. Each surfaces `{ data, isLoading, error }`. No shared rendering wrapper exists today — each page re-implements the ternary. The migration's payoff is consolidating that ternary into a single `<QueryStates />` consumer wrapper or the three primitives directly.

### luminescent-core.md is silent on lifecycle states

§5 covers components but does NOT prescribe loading/empty/error visual grammar. Per the precedent established tonight (mika#657 §5.1 / mika#654 §5.2 / mika#655 §5.3 / mika#659 §5.4), this plan adds §5.5 codifying the state-catalog grammar alongside the components.

### No Skeleton / Spinner primitives exist

Greenfield. No `Skeleton`, `Spinner`, or progress component in `packages/ui/` or `dashboard/src/components/`. Native CSS `@keyframes pulse` (Tailwind's built-in `animate-pulse`) is sufficient for v1.

### Stitch screen — visual spec

Screen `be408326efc949e49b8ab6d7c524b5f9` (generated 2026-04-27) shows a 3×2 grid:
- **Row 1 (list context):** loading skeleton table with intact header row + 6 skeleton rows; empty state with line-art icon + "Clear filters" tertiary; error state with error-tinted icon + "Retry" gradient button + "View error details ↗" link.
- **Row 2 (detail context):** loading skeleton matching detail page structure; empty sub-section variant ("No tool calls were triggered…" inline message, no button); error sub-section variant (inline error + "Retry" link + chevron details disclosure).
- **Footer:** prop signatures spelled out — `<LoadingState variant='list' | 'detail' />`, `<EmptyState message title? action? />`, `<ErrorState message? retry? detailsHref? />`.

Plan's component APIs match the Stitch footer's signatures exactly.

### Sibling-ticket overlap

- `packages/ui/CLAUDE.md` shared with mika#663/#657/#654/#655/#659 (seed-or-extend pattern; this is the 6th plan touching it).
- luminescent-core.md §5.5 follows §5.1–§5.4 precedent.

## Approach

Five changes, two layers (`packages/ui/` + `dashboard/`).

### Change 1 — Extend `luminescent-core.md` with §5.5 state-catalog grammar

**File:** `mika/docs/design/luminescent-core.md`

Add §5.5 declaring the three primitives, their use cases, and the canonical patterns. References Stitch screen `be408326efc949e49b8ab6d7c524b5f9` as the visual template.

```markdown
### 5.5 State catalog grammar (loading / empty / error)

Every list and detail surface in the dashboard renders one of three lifecycle states before the happy-path content: **loading** (fetch in progress), **empty** (request succeeded, zero results), **error** (fetch failed). The canonical primitives are `<LoadingState />`, `<EmptyState />`, `<ErrorState />` from `@senara-solutions/ui`. Hand-rolling these states (raw `Loading...` text, `text-red-400` error banners, untreated `null` returns) is a review fail.

**Visual reference:** Stitch screen `be408326efc949e49b8ab6d7c524b5f9` ("Mika State Catalog Reference") — 6 panels showing list-context and detail-context patterns.

| Primitive | Use for | Variant API |
|---|---|---|
| `<LoadingState variant="list" \| "detail" />` | Skeleton placeholder. List variant renders a header row + N skeleton rows preserving column widths. Detail variant renders metadata-strip skeleton + paragraph blocks + sub-section skeletons. | `variant` selects layout; `rows?` overrides default row count for list. |
| `<EmptyState message title? icon? action? variant? />` | Successful fetch, zero results. List context: contained within the table chrome (filter row + breadcrumbs stay visible). Detail sub-section context: compact inline message, no chrome. | `variant: 'minimal' \| 'card'` (existing); `action: { label, onClick }` (new) for "Clear filters" affordances. |
| `<ErrorState message? retry? detailsHref? variant? />` | Fetch failed. List: contained, primary "Retry" button + secondary "View error details ↗" link. Detail sub-section: compact inline, "Retry" link only. **Never expose raw stack traces — error wording must be human-shaped.** | `variant: 'list' \| 'detail-section'`; `retry: () => void` triggers refetch; `detailsHref?: string` opens log viewer or null if no detail surface available. |

**Loading skeleton contract:**
- Skeleton rectangles use `surface_container_high` (per rulebook §2 surface hierarchy) with the `animate-pulse` Tailwind utility for subtle motion. Pulse speed must be slow (~2s) — the rulebook prohibits attention-stealing animation.
- List skeleton preserves column widths from the actual table — the user sees structure forming, not a spinner-shaped void. This matches Stitch screen row-1 panel 1.
- Detail skeleton preserves the page's metadata strip + main panel + sub-section table layout.
- **Per architect Finding 5:** v1 ships uniform skeleton row heights. If post-ship UX feedback identifies specific pages where skeleton→content column-width jitter is unacceptable, follow-up trigger is to add a `columns?: { widths: string[] }` prop to `<LoadingState />`.

**Wrapper-component constraint (per architect Finding 1):**

The three primitives are consumed directly. **No `<QueryStates />` or similar wrapper component that reads query-library state (e.g., `query.isLoading`) is canonical.** A wrapper would couple `packages/ui/` to the query library's shape, breaking the library's backend-agnostic posture. If a consumer wants a convenience layer, it lives in `dashboard/src/components/`, not in `packages/ui/`. Same layer-separation argument as `<AgentFilter />` not embedding `useAgents()` (mika#655).

**Error-message conversion (per architect Finding 2 — named utility):**

Consumers MUST convert raw error objects to human-shaped strings before passing to `<ErrorState message={...} />`. The canonical conversion path is `formatApiError(error: unknown): string` exported from `@senara-solutions/ui` (added in Change 2). Three cases handled:
- Network error (`TypeError: Failed to fetch` etc.) → "Network unreachable. Check your connection."
- Server error with `detail` field (typical FastAPI/Axum error envelope) → use the detail text
- Fallback (unknown shape) → "An unexpected error occurred."

`<ErrorState message={formatApiError(error)} />` is the canonical callsite pattern. Do not pass `error.message` directly — that exposes raw internals to users. Do not invent per-page prose conventions — use the utility so 17 pages produce uniform error grammar.

**`detailsHref` constraint (per architect Finding 3):**

`detailsHref` is provided for future log-viewer linkage. v1 consumers pass `undefined`. Do not invent a destination — wait for the log-viewer surface (separate ticket) to define its URL shape and mapping convention.

**Empty state contract:**
- Surrounding chrome (filter row, breadcrumbs, page title) MUST remain visible. The empty state is contained inside the table/panel container, not a full-page takeover. This matches Stitch row-1 panel 2.
- Sub-section empty (detail context, e.g., "Tool Calls (0)") is a compact inline message in `on_surface_variant` color — no icon, no button. Matches Stitch row-2 panel 5.
- `action?: { label, onClick }` renders a primary-colored tertiary text button (e.g., "Clear filters", "Try a wider time range").

**Error state contract:**
- Error icon uses `--color-error` (#ff6e84) at low opacity — never raw red Tailwind classes (`text-red-400`).
- Error wording is human-shaped: "Failed to load sessions. The dashboard server returned 500." NOT raw error.message dumps. The component accepts a `message?: string` override; if absent, renders a generic-but-context-appropriate fallback.
- `retry: () => void` wires to the consumer's `useQuery` `refetch` function — primary gradient button per rulebook §5.
- `detailsHref?: string` opens an error-details surface (initially: log viewer URL or trace ID search). Optional; if absent, no secondary link rendered.
- **No raw stack traces, no `error.message` ternaries.** Consumers convert their error object to a human message before passing.

**Keyboard:** all action elements (`<button>` for retry, `<a>` for details) are keyboard-accessible by default. Loading skeletons are not focusable (purely structural). Empty/error icons are decorative (`aria-hidden="true"`).

**ARIA:** `<LoadingState />` includes `role="status"` and `aria-live="polite"` so screen readers announce loading. `<ErrorState />` includes `role="alert"` so screen readers escalate errors.
```

Net diff: ~50 lines added.

### Change 2 — Build `<LoadingState />` + `<ErrorState />`, extend `<EmptyState />`

**Files:**
- `mika/packages/ui/src/components/LoadingState.tsx` (new, ~80 lines)
- `mika/packages/ui/src/components/ErrorState.tsx` (new, ~70 lines)
- `mika/packages/ui/src/components/EmptyState.tsx` (existing 46 lines; +~10 lines for `action?` prop)
- `mika/packages/ui/src/utils/formatApiError.ts` (new, ~20 lines — per architect Finding 2)
- `mika/packages/ui/src/index.ts` (+3 export lines: `LoadingState`, `ErrorState`, `formatApiError`)
- `mika/packages/ui/src/theme.css` (+~5 lines — add `--color-surface-container-high` and related tokens; **verified absent during planning**, per architect Finding 6 token-before-consumer requirement)

**`<LoadingState />` API:**

```typescript
interface LoadingStateProps {
  variant: 'list' | 'detail'
  rows?: number  // override default (list = 6, detail = N/A)
  ariaLabel?: string  // defaults to "Loading"
}
```

Render shapes:
- `variant="list"`: emits a `<div role="status" aria-live="polite" aria-label={ariaLabel}>` containing a skeleton table — 6 rows of `<div className="h-10 bg-surface-container-high rounded-lg animate-pulse">` with column-width hints from a `<colgroup>` if present, otherwise uniform widths.
- `variant="detail"`: emits skeleton metadata strip (3 chips) + skeleton paragraph (3 lines of `<div className="h-4 bg-surface-container-high rounded animate-pulse">`) + skeleton sub-section table (3 rows).

**`<EmptyState />` extension (additive):**

```typescript
export interface EmptyStateProps {
  message?: string
  title?: string
  icon?: ReactNode
  variant?: 'minimal' | 'card'
  action?: { label: string; onClick: () => void }  // NEW
}
```

When `action` is set, render a tertiary text button (primary-colored, no background) below the message. Existing 6+ callsites that don't use `action` are unaffected (additive prop).

**`<ErrorState />` API:**

```typescript
interface ErrorStateProps {
  message?: string  // human-shaped fallback message
  retry?: () => void  // wires to useQuery's refetch
  detailsHref?: string
  variant?: 'list' | 'detail-section'
}
```

Render shapes:
- `variant="list"`: full-panel layout with error icon (`<AlertCircle>` from `lucide-react`, error-tinted), title "Failed to load", `message` prose, primary gradient "Retry" button (when `retry` set), tertiary "View error details ↗" link (when `detailsHref` set). `role="alert"`.
- `variant="detail-section"`: compact inline — error icon + message + "Retry" link, no title, no full-button. Matches Stitch row-2 panel 6.

**Implementation notes:**
- Both new components import `AlertCircle` and similar icons from `lucide-react` (already in dashboard's deps; verify in `packages/ui/package.json` and add as peer dep if not already).
- Skeleton rectangles use the `--color-surface-container-high` token. **Verified absent in `packages/ui/src/theme.css` during planning** (per architect Finding 6). Token-before-consumer: Change 2 adds `--color-surface-container-high` (and related surface-container tokens from rulebook §2) to `theme.css` BEFORE the components reference them. Same shape as mika#657 adding `--color-blocked` before `<StatusBadge variant="blocked">` consumed it.

**`formatApiError(error: unknown): string` utility (per architect Finding 2):**

```typescript
// packages/ui/src/utils/formatApiError.ts
export function formatApiError(error: unknown): string {
  if (error instanceof TypeError && error.message.includes('fetch')) {
    return 'Network unreachable. Check your connection.'
  }
  if (typeof error === 'object' && error !== null && 'detail' in error && typeof (error as { detail: unknown }).detail === 'string') {
    return (error as { detail: string }).detail
  }
  // React Query v4 types error as Error | null; v5 types it as unknown.
  // Guarding with instanceof keeps the utility correct across both versions
  // without pinning the library (per architect second-pass observation).
  if (error instanceof Error) {
    return error.message
  }
  return 'An unexpected error occurred.'
}
```

Four cases now (per architect second-pass): network error / server-error-envelope / Error-instance fallback / unknown fallback. The `instanceof Error` guard prevents a silent v4→v5 regression where `error.message` becomes inaccessible without a type guard. ~25 lines. Exported alongside the components.

Net diff: ~160 lines for new components + ~10 lines for EmptyState extension + 2 export lines.

### Change 3 — Migrate every dashboard list and detail page

**Files (17 dashboard pages, ~25-30 lifecycle callsites):**

For each `{ data, isLoading, error } = useQuery(...)` ternary, replace:

```tsx
{isLoading ? (
  <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
) : error ? (
  <div className="text-red-400 py-8 text-center text-sm">Error: {String(error)}</div>
) : !data || data.data.length === 0 ? (
  <EmptyState message="No sessions match your filters" />
) : (
  /* content */
)}
```

With:

```tsx
{isLoading ? (
  <LoadingState variant="list" />
) : error ? (
  <ErrorState
    variant="list"
    message="Failed to load sessions"
    message={formatApiError(error)}
    retry={() => refetch()}
  />
) : !data || data.data.length === 0 ? (
  <EmptyState
    message="No sessions match your filters"
    action={{ label: 'Clear filters', onClick: () => setSearchParams(new URLSearchParams()) }}
  />
) : (
  /* content */
)}
```

`formatApiError` is imported from `@senara-solutions/ui` alongside the three components. Consumers do not write per-page error prose — utility output is the canonical message unless a page has a domain-specific override (rare).

Per-page breakdown (from audit):
- **List pages** (variant="list" for all three states): Sessions, Tasks (5 sub-section ternaries), Timeline, LlmCalls, ToolCalls, DevRuns, TeamRuns, Agents, Traces.
- **Detail pages** (variant="detail" for top-level loading; variant="detail-section" for sub-section error/empty): TaskDetail, TeamRunDetail, DevRunDetail, LlmCallDetail, ToolCallDetail, SessionDetail, AgentDetail, TraceDetail.

mika#651's specific symptoms — `Failed to load issue` / `Failed to load PR` on DevRunDetail — are migrated to `<ErrorState variant="detail-section" message="Failed to load issue" retry={refetchIssue} detailsHref={...} />` per the canonical pattern. The "no retry, no detail" UX gap that ticket called out is closed by the new component.

Tasks page is special: 5 sub-section queries (WorkItemsSection, TeamRunTasksSection, StandaloneCallbacksSection, ScheduledSection, plus any nested) all migrate independently. Each section gets `<LoadingState variant="list" />` for its own loading state.

Net diff: ~150-200 lines reduction (substitutions are shorter than ternaries) across 17 files.

### Change 4 — Update `packages/ui/CLAUDE.md` enforcement table

**File:** `mika/packages/ui/CLAUDE.md` (sixth ticket touching it; seed-or-extend pattern).

Add three rows:

| Component | Use for | Hand-rolled forbidden | Migration status |
|---|---|---|---|
| `<LoadingState />` | All loading states (list + detail) on dashboard query consumers | Yes | Audited clean (mika#658) |
| `<EmptyState />` | All zero-result states (extended with `action`) | Yes | Audited clean (mika#658) (was Audit pending — extended in this PR) |
| `<ErrorState />` | All fetch-failure states; no raw stack traces; retry + details affordances | Yes | Audited clean (mika#658) |

Plus a note in the callsite-pattern section: lifecycle ternaries on dashboard pages must use the three primitives, never raw text. Refetch wiring uses TanStack Query's `refetch` function passed to `<ErrorState retry={...}>`.

Net diff: ~5 rows + ~10 lines callsite pattern (extending if file already exists from mika#663/#657/#654/#655/#659).

### Change 5 — Issue body update (canonical-callout convention)

**File:** mika#658 issue body (GitHub).

Replace the leading "design-needed — requires Stitch session before dispatch" callout with the new state:

```
> - **Branch:** `feat/658/dashboard-empty-loading-error-states`
> - **Plan:** `mika/docs/plans/2026-04-27-005-feat-658-dashboard-state-catalog-plan.md` (committed @ <SHA>)
> - **Stitch reference:** screen `be408326efc949e49b8ab6d7c524b5f9` ("Mika State Catalog Reference") in project `6562713725762717689` — canonical visual spec, generated 2026-04-27 via Stitch MCP. **Permanent reference for future readers** — design source-of-truth lives at this screen ID.
> - **Grooming history:** /ce:plan → mika-arch first-pass → revisions → mika-arch second-pass (GROOMED, session <id>).
> - **Status:** previously escalated as design-blocked; design has now landed (Stitch screen above). Ready to dispatch.
```

The Stitch screen ID is embedded permanently in the body (per architect Finding 7), not just in the grooming-history audit trail. Future readers find the canonical visual spec via the body's first callout, not by hunting through closed comments.

This matches the body-edit convention from mika#654 (correcting the inverted-premise body) — the leading callout reflects current verified state, not the original design-needed framing. Future readers see the design-blocked state was a momentary gate, not a permanent constraint.

Net diff: ~5 lines in the issue body.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/docs/design/luminescent-core.md` | +50 lines: §5.5 state catalog grammar |
| 2 | `mika/packages/ui/src/components/LoadingState.tsx` (new) | +~80 lines |
| 2 | `mika/packages/ui/src/components/ErrorState.tsx` (new) | +~70 lines |
| 2 | `mika/packages/ui/src/components/EmptyState.tsx` | +~10 lines: optional `action` prop |
| 2 | `mika/packages/ui/src/index.ts` | +2 lines: exports |
| 2 | `mika/packages/ui/package.json` | +1 dep if `lucide-react` not already a dep there |
| 2 | `mika/packages/ui/src/theme.css` | +1 token if `--color-surface-container-high` (etc.) not present (skeleton color tokens) |
| 3 | 17 `dashboard/src/pages/*.tsx` files | Replace lifecycle ternaries with the three primitives; ~25-30 callsites total |
| 4 | `mika/packages/ui/CLAUDE.md` | +3 rows enforcement table + callsite pattern (or seed if first; 6th ticket touching) |
| 5 | mika#658 issue body | +5 lines canonical callouts |

Estimated diff: ~400-500 lines across 22 files. Largest dashboard touch in tonight's wave.

## Tests

`@senara-solutions/ui` has no test scaffolding. Verification by:

1. **Build** — `npm run build --prefix mika/packages/ui` and `npm run build --prefix mika/dashboard` succeed.
2. **Visual** — dev server, navigate every migrated page in three test conditions:
   - Loading: throttle network in DevTools to slow-3G; observe skeleton matches column widths.
   - Empty: set filter to a no-result combo (e.g., `?agent_id=nonexistent`); observe contained empty state with "Clear filters" action.
   - Error: stop the mika-spirit; observe `<ErrorState />` with retry button. Click retry after restarting server; observe state transitions to loaded.
3. **Stitch fidelity** — operator at PR review compares migrated pages against Stitch screen `be408326efc949e49b8ab6d7c524b5f9`.
4. **Drift grep** — `grep -rn ">Loading\.\.\.<\|text-red-400\|Error:.*error" mika/dashboard/src/pages/` returns zero matches outside the new components.

## Acceptance criteria

- [ ] `mika/docs/design/luminescent-core.md` includes §5.5 declaring the state-catalog grammar (3 primitives, hand-rolled-forbidden rule, ARIA contract, error-wording-human-shaped rule, Stitch reference).
- [ ] `mika/packages/ui/src/components/LoadingState.tsx` exists with `variant: 'list' | 'detail'` API and `role="status" aria-live="polite"` semantics.
- [ ] `mika/packages/ui/src/components/ErrorState.tsx` exists with `{ message?, retry?, detailsHref?, variant }` API and `role="alert"` semantics.
- [ ] `mika/packages/ui/src/components/EmptyState.tsx` extends with optional `action?: { label, onClick }` prop; existing callsites unchanged.
- [ ] `mika/packages/ui/src/utils/formatApiError.ts` exists with three error-shape cases (network / server-detail / fallback) per architect Finding 2.
- [ ] `mika/packages/ui/src/index.ts` exports `LoadingState`, `ErrorState`, `formatApiError`, and continues to export `EmptyState`.
- [ ] `mika/packages/ui/src/theme.css` declares `--color-surface-container-high` (and related surface tokens). **Token-before-consumer verified per architect Finding 6** — token absent in pre-plan state, must ship before Change 2's components reference it.
- [ ] `<ErrorState />` callsites use `formatApiError(error)` for the message; no raw `error.message` exposure.
- [ ] All ~25-30 lifecycle ternaries across 17 dashboard pages migrate to the three primitives.
- [ ] `grep -rn ">Loading\.\.\.<\|text-red-400" mika/dashboard/src/pages/` returns zero matches.
- [ ] mika#651's `Failed to load issue` / `Failed to load PR` callouts on DevRunDetail use `<ErrorState variant="detail-section" retry={...} />` with retry wired to the relevant `useQuery` refetch.
- [ ] `mika/packages/ui/CLAUDE.md` enforcement table lists all three primitives as audited clean (mika#658).
- [ ] mika#658 issue body's "design-needed" callout is replaced with the canonical Plan/Stitch/Grooming-history callouts.
- [ ] `npm run build` succeeds in `packages/ui/` and `dashboard/`.
- [ ] Stitch fidelity check passes at PR review (operator-verified against screen `be408326efc949e49b8ab6d7c524b5f9`).
- [ ] Manual a11y check: screen reader announces loading state via `role="status"`; error state escalates via `role="alert"`.

## Out of scope

- **Adding a `<Spinner />` primitive** — skeletons cover loading; spinners are not in the Stitch spec. Follow-up trigger if non-skeleton loading affordances are needed.
- **Adding a generic `<QueryStates />` wrapper component** — would consolidate the lifecycle ternary into one `<QueryStates query={query}>{(data) => ...}</QueryStates>` consumer. Plausible v2 abstraction but introduces a render-prop pattern. Out of scope; if the migration's repeated ternary structure feels wrong, file a follow-up.
- **Migrating mika-cloud's lifecycle ternaries** — `mika-cloud` consumes `@senara-solutions/ui`; once the primitives ship, mika-cloud can adopt them as a separate effort.
- **Animated skeleton shimmer beyond `animate-pulse`** — Tailwind's `animate-pulse` provides the v1 motion. Custom shimmer (gradient sweep) is a follow-up if the rulebook prescribes it.
- **Internationalization of error messages** — plan uses English-language strings inline; i18n is a separate concern across the dashboard.
- **Replacing TanStack Query** — primitives consume `{ data, isLoading, error, refetch }` shape; query library swap is unrelated.
- **Skeleton heights/widths matching every specific table column** — v1 uses uniform skeleton rows; per-column-width matching is iteration if the visual feels off at PR review.
- **Adding a `detailsHref` log-viewer destination** — log viewer URL convention is out of scope; consumers pass `undefined` until the log-viewer surface is built. Follow-up trigger.
- **Migrating raw `null` returns in detail pages that bypass loading state** — some detail pages `return null` while `data === undefined`; replacing with `<LoadingState />` is in scope. Pages that intentionally short-circuit (e.g., on missing route param) keep their behavior.

## Risks

| Risk | Mitigation |
|---|---|
| `<EmptyState />`'s additive `action` prop changes existing render paths if the component handles `undefined` differently | Additive optional prop: when `action === undefined`, render path is identical to today. TypeScript catches missed callsites. |
| 17-file migration is the largest dashboard touch tonight; reviewer can't verify all visual outputs without dev server | PR description must include screenshots from each migrated page in all three states (loading/empty/error). AC explicitly requires this; reviewer fails on missing screenshots. |
| Skeleton row heights don't match real table row heights across all pages — visual jitter on transition | v1 uses uniform 40px row height. If specific pages have taller rows, follow-up trigger to add `rowHeight?` prop. Per-page mismatch acceptable for v1. |
| Error wording must be human-shaped, but each page's domain context differs (sessions vs LLM calls vs traces) — generic message doesn't fit | `<ErrorState message="..." />` accepts an override per consumer. Pages provide context-appropriate strings (e.g., "Failed to load sessions" vs "Failed to load LLM call telemetry"). The component falls back to a generic message only if `message` is absent. |
| `lucide-react` may not be a dep in `packages/ui` today (it's in `dashboard`) — adding it grows the library footprint | Verify in package.json before Change 2 lands. If not present, add as peer dep (consumers already have it). Bundle size impact: minimal since icons are tree-shaken. |
| `--color-surface-container-high` may not exist in `theme.css` today — skeleton color reference breaks | Verify before Change 2 lands. If absent, add as part of Change 2's theme-token additions. The luminescent-core rulebook §2 already names this token; ensuring `theme.css` reflects it is consistent with mika#657's spacing-token addition pattern. |
| TanStack Query's `refetch` function isn't available on every page's query (some use `useInfiniteQuery` or custom hooks) | Verified via grep: every list page exposes `refetch` from `useQuery`. Detail pages may use custom hooks that don't expose it; in those cases, the `<ErrorState />` `retry` prop is omitted (gracefully falls back to no-retry-button render). |
| Concurrent edits to `packages/ui/CLAUDE.md` from 5 prior tickets tonight | Standard rebase-and-resolve at merge time. Plan handles seed-or-extend like all prior tickets. |
| Stitch screen visual diverges from plan's prop API or component shape | Plan's prop signatures match Stitch screen's footer signatures verbatim (`<LoadingState variant='list'|'detail' />`, `<EmptyState message title? action? />`, `<ErrorState message? retry? detailsHref? />`). If divergence surfaces at implementation, Stitch is source of truth — primitives revised to match. |

## Sequencing

1. **Change 1 first** (luminescent-core.md §5.5 grammar). Rulebook precedes code, per #657/#654/#655/#659 precedent.
2. **Change 2 second** (build LoadingState + ErrorState; extend EmptyState; update theme tokens / package.json if needed).
3. **Change 3 third** (migrate 17 dashboard pages, ~25-30 callsites). Depends on Change 2.
4. **Change 4 fourth** (`packages/ui/CLAUDE.md` enforcement table — extend or seed).
5. **Change 5 last** (issue body update — happens during `/mika-groom-ticket` Phase 5 finalization, alongside canonical callout edits).
6. **Visual + a11y verification** (run dashboard, screenshot each migrated page in all three states, screen reader sanity check).
7. **Open PR** with screenshots + Stitch screen reference for operator visual verification.

## Verification

```bash
# Confirm rulebook extension
grep -c "5.5 State catalog grammar" mika/docs/design/luminescent-core.md  # → 1
grep -c "be408326efc949e49b8ab6d7c524b5f9" mika/docs/design/luminescent-core.md  # → 1 (Stitch reference embedded)
grep -c "No raw stack traces" mika/docs/design/luminescent-core.md  # → 1 (error-wording-human-shaped rule)

# Confirm components exist + exports
test -f mika/packages/ui/src/components/LoadingState.tsx && echo OK
test -f mika/packages/ui/src/components/ErrorState.tsx && echo OK
grep -E "LoadingState|ErrorState|EmptyState" mika/packages/ui/src/index.ts  # → 3 lines

# Confirm EmptyState extended (action prop)
grep -c "action\?:" mika/packages/ui/src/components/EmptyState.tsx  # → 1

# Confirm component a11y semantics (per AC)
grep -c 'role="status"' mika/packages/ui/src/components/LoadingState.tsx  # → 1
grep -c 'role="alert"' mika/packages/ui/src/components/ErrorState.tsx  # → 1

# Three-command migration-completeness sweep (per architect Finding 4 — gates dispatch on 17-file migration)
# 1. Hand-rolled lifecycle ternaries — expected 0 matches (all migrated to primitives)
grep -rn "isLoading ? \|isLoading &&\|? <div.*[Ll]oading" mika/dashboard/src/pages/*.tsx  # → 0 matches
# 2. Hardcoded red error styling — expected 0 matches (all migrated to <ErrorState />)
grep -rn "text-red-400\|text-red-500\|text-red-600" mika/dashboard/src/pages/*.tsx  # → 0 matches
# 3. Raw error.message exposure — expected 0 matches (consumers use formatApiError before passing)
grep -rn "error\.message\|error\?\.message" mika/dashboard/src/pages/*.tsx  # → 0 matches

# Hand-rolled "Loading..." text — secondary
grep -rn ">Loading\.\.\.<\|>Loading…<" mika/dashboard/src/pages/*.tsx  # → 0 matches

# Pages import the primitives — expected exactly 17 files
grep -rln "import.*\(LoadingState\|EmptyState\|ErrorState\).*@senara-solutions/ui" mika/dashboard/src/pages/  # → 17 files

# formatApiError is imported and used at error callsites
grep -rn "formatApiError" mika/dashboard/src/pages/*.tsx  # → ≥ 17 (one per page with an error state)

# Token verification (per architect Finding 6 — token-before-consumer)
grep -c "surface-container-high\|--color-surface-container" mika/packages/ui/src/theme.css  # → ≥ 1 after Change 2 lands

# Confirm CLAUDE.md enforcement
grep -E "LoadingState.*Audited clean.*mika#658|ErrorState.*Audited clean.*mika#658|EmptyState.*Audited clean.*mika#658" mika/packages/ui/CLAUDE.md  # → 3 matches

# Build verification
npm run build --prefix mika/packages/ui
npm run build --prefix mika/dashboard
```

## Discovery items (verified during planning)

1. **Stitch design has landed** — screen `be408326efc949e49b8ab6d7c524b5f9` generated 2026-04-27 via Stitch MCP. Visual spec exists; design-blocked status is resolved. Plan can proceed.
2. **Existing `<EmptyState />` is forward-compatible** — extending with `action?` is additive, not breaking. 6+ existing callsites unaffected.
3. **18+ pages have hand-rolled lifecycle ternaries** — universal drift across dashboard. Migration scope reflects this.
4. **mika#651's `Failed to load issue` / `Failed to load PR` symptom** is on DevRunDetail's sub-sections — the migration directly closes that UX gap with `<ErrorState variant="detail-section" retry={...} />`.
5. **luminescent-core.md is silent on lifecycle states** — adds §5.5 per the §5.1–§5.4 precedent established earlier tonight.
6. **TanStack Query `refetch` is universally available** on list pages; detail page custom hooks may not expose it — `<ErrorState />`'s `retry` is optional, so missing-refetch consumers render error without retry button.
7. **Skeleton motion uses Tailwind `animate-pulse`** — built-in, no custom CSS. Pulse speed is gentle (~2s) per rulebook anti-attention-stealing guidance.
8. **Pre-commit discovery discipline applied** — three-command sweep verifying loading text gone, red error class gone, primitive imports present.
