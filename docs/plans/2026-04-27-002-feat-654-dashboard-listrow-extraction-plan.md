---
title: "feat(ui+dashboard): extract <ListRow /> primitive (navigable/expandable/static), migrate hand-rolled rows, codify row-affordance grammar"
type: feat
status: active
date: 2026-04-27
origin: senara-solutions/mika#654
---

# Plan — `<ListRow />` extraction + dashboard migration (mika#654)

**Issue:** [mika#654](https://github.com/senara-solutions/mika/issues/654) — `Dashboard > List rows: inconsistent affordances (arrow vs non-functional chevron), factor shared ListRow into @senara-solutions/ui`
**Branch:** `feat/654/dashboard-list-rows-inconsistent-affordances`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** bug, enhancement, dashboard

## Problem (per issue body, with one premise correction)

The ticket body asks for a shared `<ListRow />` primitive with two variants (`navigable` / `expandable`) and a migration of every dashboard list page. The body cites two specific symptoms:

1. **LLM Calls list: left-side arrow `→` navigates to detail.** ✅ **Verified** at `LlmCalls.tsx:137-144` — `<Link to="/llm-calls/${row.id}">&rarr;</Link>` in the first cell.
2. **Tasks list: right-side chevron `>` does nothing.** ❌ **Verified-as-incorrect.** Tasks has **left-side** `<ChevronRight>` glyphs (`Tasks.tsx:121-125, 234-238`) that **are interactive** — they toggle row expansion (Work Items tree) and section open/close. The "right-side no-op chevron" claim is inverted; the actual artifact is left-side, on the row, fully interactive. The body's premise about Tasks needing migration to a different affordance is therefore wrong: Tasks already uses the expandable pattern correctly.

The general "drift across pages" claim **does hold**, but for different reasons than the body cites — see audit table below. Plan proceeds on verified-true subset of the body's claims.

## Audit results (verified during planning)

Inventoried all 10 list-rendering pages in `dashboard/src/pages/` (Timeline, Agents, Sessions, Traces, Tasks, LlmCalls, ToolCalls, DevRuns, TeamRuns; Traces is search UI, no list).

| Page | File:line | Wrapper | Row click | Glyph | Glyph position | Behavior | Keyboard a11y | Hover |
|---|---|---|---|---|---|---|---|---|
| Timeline | `Timeline.tsx:152-189` | `<tr>` | No | none | — | Cell links only (trace ID) | None | bg-fade |
| Agents | `Agents.tsx:47-86` | `<Link>` (card) | Yes (whole) | none | — | Card navigation | Implicit (`<a>`) | border + shadow |
| Sessions | `Sessions.tsx:125-147` | `<tr>` | No | none | — | Session ID cell link only | None | bg-fade |
| Tasks (Work Items) | `Tasks.tsx:92-188` | `<tr>` | Yes (root rows only) | `<ChevronRight>` | Left-cell | Expand tree (root rows) | None | bg-fade + cursor |
| Tasks (section header) | `Tasks.tsx:230-248` | `<button>` | Yes | `<ChevronRight>` | Left | Expand section | Implicit (`<button>`) | — |
| Tasks (callbacks/scheduled) | `Tasks.tsx:7-66` | `<tr>` | No | none | — | Label cell link | None | bg-fade |
| LlmCalls | `LlmCalls.tsx:135-183` | `<tr>` | No (arrow link) | `→` HTML entity | Left-cell | Navigate via arrow | None (link OK) | bg-fade |
| ToolCalls | `ToolCalls.tsx:138-198` | `<tr>` | Yes (whole row) | `<ChevronRight>` | Left-cell | Expand inline detail | None | bg-fade + cursor |
| DevRuns | `DevRuns.tsx:96-157` | `<tr>` | No | none | — | Label cell link | None | bg-fade |
| TeamRuns | `TeamRuns.tsx:88-121` | `<tr>` | No | none | — | Run-ID cell link | None | bg-fade |

**Three patterns observed:**
1. **Static** (Timeline, Sessions, Tasks-callbacks, DevRuns, TeamRuns): `<tr>` with consistent `hover:bg-white/[0.02]` styling, cell links navigate, row itself not interactive.
2. **Navigable** (LlmCalls): row contains a navigation glyph (`→` arrow link in first cell); the link element is the navigation primitive, not the row.
3. **Expandable** (Tasks Work Items, ToolCalls): row-level `onClick` toggles expansion, left-side chevron indicates state. Includes keyboard-accessibility gap (no `tabIndex`, no key handlers).

**Outliers:**
- **Agents** uses a `<Link>`-wrapped card layout, not a `<tr>`. Different primitive shape — not a `<ListRow />` candidate. Future `<Card />` primitive territory; **out of scope**.
- **Tasks section header** is a `<button>` with chevron, not a row. Could be a `<SectionHeader />` primitive (out of scope) or stay as-is.

**Universal gaps:**
- **No keyboard accessibility on any clickable row.** Rows with `onClick` (ToolCalls, Tasks Work Items) lack `tabIndex={0}`, `role="button"`, and Enter/Space key handlers.
- **No ARIA labels on rows.** Only LlmCalls' arrow link has `title="View details"`.
- **No existing shared row primitive.** Greenfield extraction.

`packages/ui/src/components/` exports: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge — no row component.

## Approach

Four changes, two layers (`packages/ui/` + `dashboard/`), plus rulebook extension precedent established by `mika#657`.

### Change 1 — Extend `luminescent-core.md` with row-affordance grammar (§5.2)

**File:** `mika/docs/design/luminescent-core.md` (existing rulebook)

**Why this change:** per `mika#657`'s precedent (architect Finding 1, first-pass GROOMED) — when a component API extends a silent area of the rulebook, the rulebook must declare the grammar alongside the code. The rulebook today covers status chips (§5) but is silent on list-row affordance grammar. Without naming the navigable/expandable distinction in the rulebook, every future list-page redesign re-derives row semantics differently.

Add a new subsection §5.2 (or whatever placement Vincent prefers) declaring:

```markdown
### 5.2 List row affordance grammar

Tabular and list surfaces use one of three row affordances. `<ListRow />` from `@senara-solutions/ui` is the canonical rendering primitive; hand-rolling `<tr>` or row-level `onClick` outside this primitive is forbidden.

| Variant | Visual | Behavior | When to use |
|---|---|---|---|
| `static` | Cell-level links navigate; row itself not interactive | Row click is a no-op | Tabular data where individual cells (IDs, names, links) navigate to context-specific destinations |
| `navigable` | Whole row is clickable, optional `→` glyph in first cell | Row click navigates to detail page | List pages where every row maps 1:1 to a detail page (LLM Calls, Tool Calls list view) |
| `expandable` | Whole row is clickable, left-side chevron indicates state | Row click toggles inline expansion (more details, child rows) | Hierarchical or detail-rich rows where inline expansion is more useful than navigation (Tasks tree, Tool Calls detail expansion) |

**Glyph conventions:**
- `navigable`: optional `→` arrow on the left, indicates "click to enter."
- `expandable`: `chevron-right` collapsed → `chevron-down` expanded, on the left.
- `static`: no glyph; row is not advertising click affordance.

**Keyboard interaction model (per architect Finding 2 — name the contract, not just attributes):**
- **Navigable:** Enter triggers navigation (same as click). Space optionally triggers (consistent with `<a>` semantics — Enter only, Space scrolls). Tab moves focus on/off the row. No expansion or collapse semantics.
- **Expandable:** Enter or Space toggles expansion. Escape collapses if currently expanded (no-op if collapsed). Tab moves focus on/off the row. Focused state must be visually distinct (`focus-visible` ring per design tokens).
- **Static:** not focusable; not keyboard-interactive. Only nested links are keyboard-navigable via Tab.
- **Nested-element guard:** for navigable/expandable rows, the row's keyboard handler triggers only when the focus target is the row itself (not a child link/button). Implementation uses `e.target.closest('[data-list-row]') === e.currentTarget` or equivalent. Consumers do NOT need to call `e.stopPropagation()` on nested links — the component handles it.

**ARIA:**
- `navigable`: `role="link"` (or rendered as `<a>` directly) with `aria-label` describing destination.
- `expandable`: `role="button"` with `aria-expanded={true|false}` and `aria-label` describing the expansion target.
- `static`: no role attribute; row is purely structural.
```

**Layering discipline:** rulebook references the variant names and behaviors; concrete CSS values (hover color, padding) live in `theme.css` and the component implementation. No hex literals in the rulebook.

Net diff: ~30 lines added to `luminescent-core.md`.

### Change 2 — Build `<ListRow />` in `@senara-solutions/ui`

**New file:** `mika/packages/ui/src/components/ListRow.tsx`

Component shape:

```typescript
interface ListRowProps {
  variant: 'static' | 'navigable' | 'expandable'
  children: React.ReactNode  // table cells
  // navigable
  to?: string                // path for variant='navigable'; renders the row as a Link
  onClick?: () => void       // alternative to `to` if navigation is imperative (e.g., useNavigate)
  // expandable
  isExpanded?: boolean
  onToggle?: () => void
  // accessibility
  ariaLabel?: string
}
```

**Glyph determined by variant (per architect Finding 4, first-pass):** the `glyph` prop has been dropped from the API. Glyph is determined by variant:
- `navigable` → `→` arrow rendered in a leading `<td>` (auto-injected by component)
- `expandable` → chevron-right (collapsed) / chevron-down (expanded) rendered in a leading `<td>`
- `static` → no glyph

YAGNI argument: zero migration targets need a non-default glyph. Allowing variant/glyph independence (e.g., `<ListRow variant="navigable" glyph="none">`) creates a silent-divergence risk — a future developer can deviate from the rulebook grammar without touching `luminescent-core.md`. Same failure mode mika#657 Finding 1 prevents. If a future use case requires a non-default glyph, the correct path is to extend the rulebook first, not add an API prop.

Render shape:
- All variants emit `<tr data-list-row className="hover:bg-white/[0.02] transition-colors">` with the cells passed as `children` (plus the auto-injected glyph cell for navigable/expandable).
- `navigable` adds `tabIndex={0}`, `role="link"`, `onKeyDown` (Enter triggers `to` or `onClick`), and `cursor-pointer`.
- `expandable` adds `tabIndex={0}`, `role="button"`, `aria-expanded`, `onKeyDown` (Enter/Space toggles, Escape collapses if expanded), and `cursor-pointer`.
- `static` adds none of the above — pure structural row with consistent hover.
- **Nested-element guard (per architect Finding 5):** keyboard and click handlers check `e.target.closest('[data-list-row]') === e.currentTarget` (or equivalent rowRef check) before triggering. Nested links/buttons inside expandable rows therefore don't accidentally activate the row handler. Consumers don't need `e.stopPropagation()` — the component handles it.

**Why this shape:**
- `static` exists primarily to make the canonical-primitive enforcement complete — every `<tr>` in a list page passes through `<ListRow />`, so any `<tr>` outside it becomes a review fail. The visual is identical to today (className-equivalent), so no UX change.
- `navigable` covers LlmCalls arrow-glyph + future cases where whole-row click is the right pattern.
- `expandable` covers ToolCalls + Tasks Work Items, with keyboard accessibility added.

Export: add `<ListRow />` to `packages/ui/src/index.ts`. Net diff: ~80 lines for the component + 1 line for the export.

### Change 3 — Migrate dashboard list pages to `<ListRow />`

**Files (8 list-rendering callsites across 7 pages):**

| Callsite | Migrates to |
|---|---|
| `Timeline.tsx:152-189` | `<ListRow variant="static">` wrapping cells |
| `Sessions.tsx:125-147` | `<ListRow variant="static">` |
| `Tasks.tsx:7-66` (TaskRow) | `<ListRow variant="static">` |
| `Tasks.tsx:92-188` (ExpandableTaskRow) | `<ListRow variant="expandable" isExpanded={isExpanded} onToggle={toggle} ariaLabel={`Toggle subtasks of ${task.label}`}>` (root rows only) + `variant="static"` for non-root tree depth rows |
| `LlmCalls.tsx:135-183` | `<ListRow variant="navigable" to={`/llm-calls/${row.id}`} ariaLabel={`View details for LLM call ${row.id}`}>` (arrow glyph auto-injected by variant) |
| `ToolCalls.tsx:138-198` | `<ListRow variant="expandable" isExpanded={isOpen} onToggle={() => toggleExpand(row.id)} ariaLabel={`Toggle details for tool call ${row.id}`}>` |
| `DevRuns.tsx:96-157` | `<ListRow variant="static">` |
| `TeamRuns.tsx:88-121` | `<ListRow variant="static">` |

**Out of scope:**
- `Agents.tsx:47-86` — card layout via `<Link>` wrapper, not a `<tr>`. Future `<Card />` primitive.
- `Tasks.tsx:230-248` — section header `<button>`, not a row. Future `<SectionHeader />` primitive.

After migration, `grep -rn "<tr.*hover:bg-white" mika/dashboard/src/pages/` should return zero matches outside `<ListRow />`'s output (which bubbles up only inside `packages/ui/`).

Net diff: ~150-200 lines across 7 dashboard files; per-callsite change is roughly substituting the `<tr ...>` wrapper line and removing inline `onClick`/`tabIndex` boilerplate.

### Change 4 — Document canonical-primitive enforcement in `packages/ui/CLAUDE.md`

**File:** `mika/packages/ui/CLAUDE.md` (does not exist yet — `mika#663` and `mika#657` plans both seed/extend it).

If `mika#663` or `mika#657` ships first, extend the existing enforcement table with:

| Component | Use for | Hand-rolled forbidden | Migration status |
|---|---|---|---|
| `<ListRow />` | All `<tr>` row rendering in list/table surfaces (static, navigable, expandable) | Yes | Audited clean (mika#654) |

If this PR ships before either of those, seed the file using the same shape (`mika#663`'s plan describes the seed shape).

Net diff: +1 row if extending, ~50 lines if seeding.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/docs/design/luminescent-core.md` | +30 lines: §5.2 row-affordance grammar (variants, glyphs, keyboard requirements, ARIA) |
| 2 | `mika/packages/ui/src/components/ListRow.tsx` (new) | +~80 lines: component with three variants, glyph rendering, keyboard handlers, ARIA |
| 2 | `mika/packages/ui/src/index.ts` | +1 line: export `ListRow` |
| 3 | `mika/dashboard/src/pages/Timeline.tsx` | Wrap row rendering in `<ListRow variant="static">` |
| 3 | `mika/dashboard/src/pages/Sessions.tsx` | Same |
| 3 | `mika/dashboard/src/pages/Tasks.tsx` | TaskRow → static; ExpandableTaskRow → expandable (root) + static (non-root) |
| 3 | `mika/dashboard/src/pages/LlmCalls.tsx` | Replace first-cell arrow link + `<tr>` wrapper with `<ListRow variant="navigable" to=... glyph="arrow">` |
| 3 | `mika/dashboard/src/pages/ToolCalls.tsx` | Replace `<tr onClick={...}>` + chevron cell with `<ListRow variant="expandable" isExpanded={isOpen} onToggle={...}>` |
| 3 | `mika/dashboard/src/pages/DevRuns.tsx` | Wrap row rendering in `<ListRow variant="static">` |
| 3 | `mika/dashboard/src/pages/TeamRuns.tsx` | Same |
| 4 | `mika/packages/ui/CLAUDE.md` | +1 row in enforcement table (or seed if first) |

Estimated diff: ~250-300 lines across 11 files.

## Tests

`@senara-solutions/ui` has no test scaffolding (verified). Verification by:

1. **Build verification** — `npm run build --prefix mika/packages/ui` and `npm run build --prefix mika/dashboard` both succeed.
2. **Visual verification** — `npm run dev:dashboard` (per root CLAUDE.md), navigate to each migrated page:
   - Static rows: Timeline, Sessions, DevRuns, TeamRuns, Tasks (callbacks/scheduled) — visual identical to before; cell links work; row is not focusable.
   - Navigable: LlmCalls — whole row is focusable (Tab cycles through), Enter navigates, glyph hover changes color.
   - Expandable: ToolCalls, Tasks Work Items — row is focusable, Enter/Space toggles, chevron rotates, aria-expanded updates.
3. **Drift grep** — `grep -rn "<tr.*hover:bg-white" mika/dashboard/src/pages/` should return zero matches outside what `<ListRow />` produces in the bundle.
4. **Keyboard a11y manual check** — Tab through each migrated page; focus ring visible on navigable/expandable rows; Enter/Space activates correctly; nested links inside expandable rows don't accidentally trigger row toggle (verify `stopPropagation`).
5. **Tasks-tree edge case** — confirm root-row expand/collapse still works AND nested-link click on `task.label` still navigates to detail (the existing `e.stopPropagation()` pattern must survive migration).

## Acceptance criteria

- [ ] `mika/docs/design/luminescent-core.md` includes §5.2 declaring row-affordance grammar (static/navigable/expandable, glyph conventions, keyboard requirements, ARIA).
- [ ] `mika/packages/ui/src/components/ListRow.tsx` exists with three variants and full keyboard/ARIA support; **no `glyph` prop** (glyph determined by variant).
- [ ] `mika/packages/ui/src/index.ts` exports `ListRow`.
- [ ] Issue body of mika#654 has been edited to correct the inverted Tasks-chevron premise (per architect Finding 1, conditional dispatch-blocker resolved at finalization).
- [ ] All seven dashboard list pages (Timeline, Sessions, Tasks, LlmCalls, ToolCalls, DevRuns, TeamRuns) wrap their `<tr>` rendering in `<ListRow />` with the appropriate variant per audit table.
- [ ] `Tasks.tsx` ExpandableTaskRow uses `<ListRow variant="expandable">` for root rows; non-root tree-depth rows use `<ListRow variant="static">` (or fold into the same component if structurally cleaner).
- [ ] `LlmCalls.tsx` uses `<ListRow variant="navigable" to=... glyph="arrow">` and the dedicated arrow-cell `<Link>` is removed (the variant handles glyph rendering).
- [ ] `ToolCalls.tsx` uses `<ListRow variant="expandable" ...>` and the explicit `onClick` on `<tr>` plus inline chevron cell are removed.
- [ ] `grep -rn "<tr.*hover:bg-white" mika/dashboard/src/pages/` returns zero matches.
- [ ] `grep -rn "onClick.*toggleExpand\|onClick.*setOpen\|onClick.*toggle" mika/dashboard/src/pages/*.tsx | grep "<tr"` returns zero matches (no row-level `onClick` outside `<ListRow />`).
- [ ] Manual keyboard a11y check: Tab/Enter/Space work correctly on every migrated row; nested-link click does not trigger row activation; aria-expanded reflects state on expandable rows.
- [ ] `mika/packages/ui/CLAUDE.md` enforcement table lists `<ListRow />` with `Audited clean (mika#654)`.
- [ ] `npm run build` succeeds in `packages/ui/` and `dashboard/`.

## Out of scope

- **Agents card layout (`Agents.tsx:47-86`)** — `<Link>`-wrapped card, not a `<tr>`. Different primitive shape. Future `<Card />` ticket if/when card-layout drift surfaces; not a `<ListRow />` candidate.
- **Tasks section header (`Tasks.tsx:230-248`)** — `<button>` with chevron, not a row. Future `<SectionHeader />` if collapse-section primitives proliferate; out of scope here.
- **Migrating non-list `<tr>` instances** — detail-page metadata tables (e.g., LlmCallDetail's metadata rows) are not list rows; they don't migrate to `<ListRow />`. The grammar is for tabular *list* surfaces.
- **Replacing the existing `<MetadataRow />` in `dashboard/src/components/`** — that's label/value layout for detail pages, different concept.
- **Adding tests for `<ListRow />`** — `@senara-solutions/ui` has no test scaffolding today; adding it is a separate ticket.
- **Migrating mika-cloud's row rendering** — mika-cloud consumes `@senara-solutions/ui` but its own page-level migrations are a separate effort.
- **Body's "right-side chevron no-op" framing for Tasks** — verified incorrect during planning. The plan does NOT redesign Tasks' chevron behavior; it's already correct (left-side, expandable, interactive).

## Risks

| Risk | Mitigation |
|---|---|
| `<ListRow variant="expandable">` API doesn't cleanly fit Tasks Work Items' tree-depth `indent` prop (non-root rows pass `indent` for visual nesting) | The migration uses `static` variant for non-root rows and `expandable` only for root rows. Nested rows still receive `indent` via children, no API change needed. If this collapses awkwardly, add `indent?: number` prop to `<ListRow />` in iteration. |
| Wrapping cells under `<ListRow />` shifts the rendered DOM tree; existing CSS selectors targeting `<tr>` directly may break | All hover styling moves into `<ListRow />`'s output `<tr>`; no consumer-level CSS targets `<tr>` directly (verified by grep — no `tr {`, `tr.someclass`, or descendant selectors in `dashboard/src/index.css`). Risk is low. |
| Keyboard handlers on expandable rows conflict with nested `<Link>` clicks (existing `e.stopPropagation()` pattern in Tasks) | `<ListRow />`'s key handler reads `e.target` and only triggers if the target is the row itself, not a descendant. Plus consumers continue to use `e.stopPropagation()` on nested links. Test this explicitly in manual a11y check. |
| Tasks ExpandableTaskRow root-row test (line 117 `isRoot ? toggle : undefined`) doesn't translate cleanly to a single variant | The migration uses two `<ListRow />` instances per row depth: `expandable` for root, `static` for descendants. Adds slight render-path branching but matches existing semantics exactly. |
| Migration touches 7 files in dashboard; reviewer can't verify all visual outputs without running the dev server | PR description must include screenshots from each migrated page. AC explicitly requires the manual visual check. Reviewer fails AC on missing screenshots. |
| `<ListRow />` ships with `static` variant that is identical to a plain `<tr>` — premature factoring? | The factoring is justified by enforcement (any `<tr>` in a list page outside `<ListRow />` becomes a review fail). Net visual is identical, but the constraint catches future drift. Architect can challenge if this is too premature. |
| `glyph` API is overdetermined (`'arrow' | 'chevron' | 'none'` vs. variant-specific defaults) | Defaults are sensible (navigable→arrow, expandable→chevron, static→none); explicit prop is for cases where a variant uses a non-default glyph (rare, but possible). If unused after migration, drop in iteration. |

## Sequencing

1. **Pre-grooming step — body edit (per architect Finding 1, conditional dispatch-blocker).** Before plan-on-branch dispatch, the issue body's Tasks-chevron premise must be corrected. Replace the "right-side no-op chevron" symptom description with the verified finding: Tasks has left-side interactive chevrons at `Tasks.tsx:121-125` (root rows) and `:234-238` (section header); the affordance concern is keyboard-a11y absence and shared-primitive absence across all seven pages, not a non-functional glyph. Issue-as-versioned-contract principle (mika-platform#52 convention): the body is the durable artifact; future readers must see the verified description, not the inverted premise. Body-edit happens during Phase 5 (finalization) of `/mika-groom-ticket`, alongside the canonical-callout edits.
2. **Change 1 first** (luminescent-core.md §5.2 grammar). Rulebook declares affordances before code consumes them. Architectural ordering precedent from `mika#657`.
3. **Change 2 second** (`<ListRow />` component). Implements grammar from Change 1.
4. **Change 3 third** (migrate 7 dashboard pages). Depends on Change 2.
5. **Change 4 last** (`packages/ui/CLAUDE.md` enforcement table — seed or extend depending on prior tickets' ship state).
6. **Visual + keyboard a11y verification** (run dashboard, screenshot each migrated page, Tab through each, verify Enter/Space/Escape).
7. **Open PR** cross-referencing `mika#654` with screenshots and a note that the body has been corrected (the original premise was inverted).

## Verification

Per architect Finding 7, the verification block names the complete discovery commands with expected output shapes — not just structural greps. Three-command discipline (mika#663 / #657 precedent applied to row-migration scope):

```bash
# Confirm rulebook extension declares the grammar
grep -c "5.2 List row affordance grammar" mika/docs/design/luminescent-core.md  # → 1
grep -c "Hand-rolling.*onClick.*outside this primitive is forbidden" mika/docs/design/luminescent-core.md  # → 1
grep -c "Enter or Space toggles expansion" mika/docs/design/luminescent-core.md  # → 1 (keyboard contract per Finding 2)

# Confirm component exists and is exported with three-variant API (no glyph prop)
test -f mika/packages/ui/src/components/ListRow.tsx && echo "OK"
grep -c "ListRow" mika/packages/ui/src/index.ts  # → ≥ 1
grep "variant: 'static' | 'navigable' | 'expandable'" mika/packages/ui/src/components/ListRow.tsx  # → match
grep -c "glyph" mika/packages/ui/src/components/ListRow.tsx  # → 0 in interface (per Finding 4: glyph determined by variant, not a prop)

# Three-command pre-commit discovery sweep — row migration completeness
# 1. Clickable raw <tr> elements (should be 0 after migration)
grep -rn "onClick.*row\|<tr.*onClick" mika/dashboard/src/pages/*.tsx  # → 0 matches

# 2. Hand-rolled keyboard a11y (tabIndex, onKeyDown, role="button"/"link") in dashboard pages
#    (after migration, all of these live inside <ListRow />, not in page code)
grep -rn 'tabIndex=\|onKeyDown=\|role="button"\|role="link"' mika/dashboard/src/pages/*.tsx  # → 0 matches

# 3. Raw <tr> outside <ListRow /> bubble-up
#    Expected matches: out-of-scope detail-page metadata tables only; each named in PR description
grep -rn "<tr" mika/dashboard/src/pages/*.tsx | grep -v "ListRow"  # → matches only out-of-scope detail tables

# Confirm structural drift detector
grep -rn "<tr.*hover:bg-white" mika/dashboard/src/pages/  # → 0 matches

# Confirm ListRow imports in migrated files
grep -l "import.*ListRow.*@senara-solutions/ui" mika/dashboard/src/pages/  # → ≥ 7 files

# Confirm packages/ui/CLAUDE.md lists ListRow as audited clean
grep "ListRow.*Audited clean.*mika#654" mika/packages/ui/CLAUDE.md  # → match

# Build verification
npm run build --prefix mika/packages/ui
npm run build --prefix mika/dashboard
```

## Discovery items (verified during planning)

1. **Body's Tasks-chevron premise is inverted.** Tasks has left-side chevrons that ARE interactive (tree expand/collapse). The "right-side no-op chevron" claim isn't reproducible in the current code. Plan corrects this in the audit section so the architect doesn't waste cycles re-verifying.
2. **Three patterns observed, not two.** Body proposed `navigable | expandable`; audit found a third needed: `static`. Without `static`, half the pages (Timeline, Sessions, DevRuns, TeamRuns, Tasks-callbacks) can't migrate without changing UX (forcing whole-row click where today only cells link). Three-variant API preserves existing UX while enforcing the canonical primitive.
3. **Keyboard accessibility is universally absent on clickable rows.** Migration adds `tabIndex`, `role`, `onKeyDown` to navigable + expandable variants — a real UX improvement.
4. **Agents card layout is not a `<ListRow />` candidate.** Different primitive (Card, not Row). Out-of-scope but explicitly named so the body's "every list page" framing doesn't catch it as missed scope.
5. **`packages/ui/CLAUDE.md` is shared-artifact territory** with `mika#663` and `mika#657`. Seed-or-extend pattern handles concurrent branches.
6. **No existing shared row primitive in `packages/ui/` or `dashboard/src/components/`.** Greenfield extraction; no migration of an existing component.
7. **Pre-commit discovery discipline applies:** the audit verifies callsite count + verifies the body's specific Tasks claim against current source. Two-part discipline (#663 / #657 precedent).
