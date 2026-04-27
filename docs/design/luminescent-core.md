# The Luminescent Core — Mika Design System

**Status:** Active — the single rulebook for all Mika visual surfaces.
**Scope:** Observability Dashboard, Cloud Console, Landing Page, and the shared `@senara-solutions/ui` component library.
**Owner:** Vincent. Updates land as direct commits to main; this rulebook is not relitigated through PRs.
**Companion:** [`north-star.md`](./north-star.md) — the WHY this rulebook exists.
**Origin:** Authored by Vincent during iteration on the Mika Cloud Console (Stitch project `7456518174288683643`, March 2026). Promoted to ecosystem-wide rulebook 2026-04-25.

---

## 1. Overview & Creative North Star

The Creative North Star for this design system is **"The Digital Curator."**

We are moving away from the cluttered, "dashboard-heavy" aesthetic common in AI platforms. Instead, we treat the management of AI agents as a high-end editorial experience. The interface should feel like a dark, quiet gallery where the AI's intelligence is the primary exhibit.

To break the "template" look, we employ **Intentional Asymmetry**. Rather than a rigid 12-column centered grid, we lean into expansive left-aligned headings (using `display-lg`) contrasted against compact, high-density data modules. We use overlapping elements — such as a `primary_dim` glow bleeding behind a `surface_container` card — to create a sense of three-dimensional space that feels bespoke and premium.

---

## 2. Colors & Atmospheric Depth

The color palette is designed to simulate a deep, infinite void where UI elements "float" rather than sit.

### The "No-Line" Rule

Standard 1px solid borders are strictly prohibited for sectioning. Definition must be achieved through **Tonal Shifting**. Use `surface_container_low` for large section backgrounds resting on the `background` (`#0c0e11`) to create a natural, soft boundary.

### Surface Hierarchy & Nesting

Treat the UI as a series of nested physical layers.

- **Base:** `background` (#0c0e11)
- **Primary Layout Blocks:** `surface_container` (#171a1d)
- **Interactive Cards:** `surface_container_high` (#1d2024)
- **Popovers/Modals:** `surface_container_highest` (#23262a)

### The "Glass & Gradient" Rule

To elevate beyond a "flat" dark mode, use Glassmorphism for floating navigation and action bars. Apply `surface_variant` at 60% opacity with a `backdrop-filter: blur(20px)`.

### Signature Textures

Main CTAs must use a linear gradient: `primary` (#ada3ff) to `primary_dim` (#715eeb) at a 135-degree angle. This provides a "soul" to the interface that a flat hex code cannot replicate.

### Full Token Reference

| Token | Hex |
|---|---|
| `background` / `surface` / `surface_dim` | `#0c0e11` |
| `surface_container_lowest` | `#000000` |
| `surface_container_low` | `#111317` |
| `surface_container` | `#171a1d` |
| `surface_container_high` | `#1d2024` |
| `surface_container_highest` / `surface_variant` | `#23262a` |
| `surface_bright` | `#292c31` |
| `on_background` / `on_surface` | `#e8e8ec` |
| `on_surface_variant` | `#aaabaf` |
| `outline` | `#747579` |
| `outline_variant` | `#46484b` |
| `primary` | `#ada3ff` |
| `primary_dim` | `#715eeb` |
| `primary_container` / `primary_fixed` | `#9f93ff` |
| `primary_fixed_dim` | `#9182ff` |
| `secondary` / `secondary_dim` | `#9d8fff` |
| `secondary_container` | `#4434a0` |
| `tertiary` | `#f6f9ff` |
| `error` | `#ff6e84` |
| `error_dim` | `#d73357` |
| `error_container` | `#a70138` |
| `surface_tint` | `#ada3ff` |
| `inverse_surface` | `#f9f9fd` |

Override neutrals: `#0d0f12`. Override primary: `#7c6af7`. Override secondary: `#9d8fff`. Override tertiary: `#e8ecf2`.

---

## 3. Typography: The Editorial Voice

Our typography establishes an authoritative yet breathable hierarchy. We pair the geometric warmth of **Plus Jakarta Sans** for interface elements with the technical precision of **JetBrains Mono** (used for agent logs, IDs, and code snippets).

- **Display Scales (`display-lg`, `display-md`):** Use these sparingly for hero headers. Set with `-0.04em` letter-spacing to create a "tight," premium editorial feel.
- **Headline to Body Ratio:** Headlines use `on_surface` (#e8e8ec) for maximum contrast, while body text uses `on_surface_variant` (#aaabaf). This 30% drop in luminance ensures the user's eye is pulled immediately to the most important information.
- **Labels:** Always use `label-md` or `label-sm` in uppercase with `0.05em` letter-spacing when paired with AI status indicators to signify "System Metadata."

Font assignments: `font` = `headlineFont` = `bodyFont` = `labelFont` = **Plus Jakarta Sans**. Code/IDs/logs = **JetBrains Mono**.

---

## 4. Elevation & Depth: Tonal Layering

We reject traditional drop shadows in favor of **Ambient Light**.

- **The Layering Principle:** Instead of a shadow, place a `surface_container_highest` card inside a `surface_container_low` area. The delta in hex value creates enough contrast to signify "lift" without visual clutter.
- **Luminescent Shadows:** When an element must float (e.g., a primary Modal), use a shadow tinted with `surface_tint`.
  - *Spec:* `box-shadow: 0 20px 40px rgba(124, 106, 247, 0.08);`
- **The "Ghost Border" Fallback:** If accessibility requires a stroke, use the `outline_variant` token at 20% opacity. This creates a "barely-there" guide that maintains the minimal aesthetic.

---

## 5. Components: Precision Primitives

### Buttons

- **Primary:** Gradient fill (`primary` to `primary_dim`), `xl` (1.5rem) roundedness. No border.
- **Secondary:** Ghost style. `outline_variant` at 20% opacity border. On hover, transition to `secondary_container` background.
- **Tertiary:** Text-only using `primary` color, no background, for low-priority actions.

### Cards & Lists (The Divider-Free Approach)

Forbid 1px divider lines. Separate list items using the **Spacing Scale**:

- Use `spacing-4` (1rem) of vertical white space between items.
- Or, use alternating background shifts between `surface_container` and `surface_container_low`.

### AI Agent Status Chips

- Use `surface_bright` as the background with a 2px inner "glow" dot using the `primary` token to signify an active agent.

### 5.1 Multi-state status grammar

The active-agent chip above is the canonical surface form. For surfaces requiring multi-state status indication (success/failed operations, pending/blocked task states, info/neutral classifications), the design system declares six variants. `<StatusBadge />` from `@senara-solutions/ui` is the canonical rendering primitive for this grammar.

| Variant | Token | Semantic meaning |
|---|---|---|
| `success` | `--color-success` | Operation completed successfully; agent active; positive terminal state |
| `warning` | `--color-warning` | Degraded, paused, pending — caution but not failure; can resume |
| `error` | `--color-error` | Operation failed; intervention required |
| `info` | `--color-accent` | Active operation in progress; informational/in-motion state |
| `neutral` | `--color-muted` | Cancelled, archived, or stateless; no active signal |
| `blocked` | `--color-blocked` | External-dependency wait; visually distinct from `warning` to preserve at-a-glance signal when paired with pending/suspended states in tabular contexts |

Labels render UPPERCASE with `tracking-wide` (`0.05em` letter-spacing) per §3 typography. The active-agent chip above is a specialized form of `success` with the `dotPulse` modifier.

**Hand-rolled status pills are forbidden.** Any new surface code rendering its own status pill (success/error inline indicators, custom colored dots with text) is a review fail. Use `<StatusBadge variant="..." label="..." />` from `@senara-solutions/ui`. For task-domain status (`pending`, `in_progress`, `completed`, etc.), use `<TaskStatusBadge status={...} />` which delegates to `<StatusBadge />` with the canonical task→variant mapping.

### 5.2 List row affordance grammar

Tabular and list surfaces use one of three row affordances. `<ListRow />` from `@senara-solutions/ui` is the canonical rendering primitive; hand-rolling `<tr>` with row-level `onClick` outside this primitive is forbidden.

| Variant | Visual | Behavior | When to use |
|---|---|---|---|
| `static` | Cell-level links navigate; row itself not interactive | Row click is a no-op | Tabular data where individual cells (IDs, names, links) navigate to context-specific destinations |
| `navigable` | Whole row is clickable, `→` glyph in first cell | Row click navigates to detail page | List pages where every row maps 1:1 to a detail page |
| `expandable` | Whole row is clickable, left-side chevron indicates state | Row click toggles inline expansion (more details, child rows) | Hierarchical or detail-rich rows where inline expansion is more useful than navigation |

**Glyph conventions:**
- `navigable`: `→` arrow on the left, indicates "click to enter."
- `expandable`: `chevron-right` collapsed → `chevron-down` expanded, on the left.
- `static`: no glyph; row is not advertising click affordance.

**Keyboard interaction model:**
- **Navigable:** Enter triggers navigation (same as click). Tab moves focus on/off the row.
- **Expandable:** Enter or Space toggles expansion. Escape collapses if currently expanded. Tab moves focus on/off the row. Focused state must be visually distinct (`focus-visible` ring per design tokens).
- **Static:** not focusable; not keyboard-interactive. Only nested links are keyboard-navigable via Tab.
- **Nested-element guard:** for navigable/expandable rows, the row's keyboard handler triggers only when the focus target is the row itself (not a child link/button).

**ARIA:**
- `navigable`: `role="link"` with `aria-label` describing destination.
- `expandable`: `role="button"` with `aria-expanded={true|false}` and `aria-label` describing the expansion target.
- `static`: no role attribute; row is purely structural.

### 5.3 Filter affordance grammar

Dashboard list surfaces use one of two filter primitives. `<SelectFilter />` from `@senara-solutions/ui` is the canonical primitive for categorical selection (one-of-N from a fixed or fetched option set). `<AgentFilter />` is a specialization that fetches agents via consumer-injected `agents` prop internally. Hand-rolling `<select>` or filter-shaped `<input>` with categorical options outside these primitives is forbidden.

| Primitive | Use for | Options | Example surfaces |
|---|---|---|---|
| `<SelectFilter />` | Categorical filter (one-of-N) where the option set is known | Static array (`{ label, value }`) or fetched array | channel_type, event_type, success, status |
| `<AgentFilter />` | Specialized agent selection (one agent from the active set) | Consumer-injected `agents` prop | Sessions, Timeline, LlmCalls, ToolCalls |

**Free-text filters** (e.g., session ID lookup, trace ID lookup, free-text search over content) remain native `<input type="text">` with consistent styling. They are not categorical and do not migrate to `<SelectFilter />`. A `<TextFilter />` primitive may emerge if free-text styling drift surfaces.

**Agent selection: exact `agent_id` match is the canonical pattern (named design decision).** Three of four pages with agent filters already render a dropdown; `<AgentFilter />` canonicalizes that pattern. Substring or partial match on agent name is not a supported filter affordance **in v1**. If agent set size grows beyond ~20, or if user feedback reveals search need, the designated follow-up path is `<AutocompleteFilter />` — not extending `<AgentFilter />` with a substring-match prop, and not re-introducing free-text on a single page.

**Visual contract:**
- Both primitives render a single dropdown with the canonical filter styling (border, rounded-lg, focus ring per design tokens).
- An empty / "all" option is always the first item, labeled to the consumer's preference (`All Channels`, `All Agents`, `All Statuses`).
- Selected value reflects URL state via `useSearchParamsFilter`'s `updateFilter`.

**Keyboard:** native `<select>` keyboard semantics (Tab focuses, Up/Down navigates options, prefix typing jumps to matching option, Esc closes). v1 does not implement custom combobox / type-ahead; if option-set growth or search-required UX surfaces, that's the trigger for an `<AutocompleteFilter />` follow-up.

**ARIA:** `aria-label` describing the filter dimension (e.g., `aria-label="Filter by agent"`). Native `<select>` provides the rest.

### Input Fields

- Background: `surface_container_lowest`.
- Border: `outline_variant` at 10% opacity.
- Focus State: Border transitions to `primary` with a 4px outer `primary_dim` blur at 10% opacity.

### Additional Component: The "Pulse" Console

For AI platforms, we introduce the **Console Component**: A `surface_container_highest` block using **JetBrains Mono** text. It features no borders but utilizes a "Purple Glow" (a `primary` radial gradient at 5% opacity) in the top-right corner to suggest the agent is "thinking."

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
- v1 ships uniform skeleton row heights. If post-ship UX feedback identifies specific pages where skeleton→content column-width jitter is unacceptable, follow-up trigger is to add a `columns?: { widths: string[] }` prop to `<LoadingState />`.

**Wrapper-component constraint:**

The three primitives are consumed directly. **No `<QueryStates />` or similar wrapper component that reads query-library state (e.g., `query.isLoading`) is canonical.** A wrapper would couple `packages/ui/` to the query library's shape, breaking the library's backend-agnostic posture. If a consumer wants a convenience layer, it lives in `dashboard/src/components/`, not in `packages/ui/`.

**Error-message conversion:**

Consumers MUST convert raw error objects to human-shaped strings before passing to `<ErrorState message={...} />`. The canonical conversion path is `formatApiError(error: unknown): string` exported from `@senara-solutions/ui`. Three cases handled:
- Network error (`TypeError: Failed to fetch` etc.) → "Network unreachable. Check your connection."
- Server error with `detail` field (typical FastAPI/Axum error envelope) → use the detail text
- Error instance → use the error message
- Fallback (unknown shape) → "An unexpected error occurred."

`<ErrorState message={formatApiError(error)} />` is the canonical callsite pattern. Do not pass `error.message` directly — that exposes raw internals to users. Do not invent per-page prose conventions — use the utility so all pages produce uniform error grammar.

**`detailsHref` constraint:**

`detailsHref` is provided for future log-viewer linkage. v1 consumers pass `undefined`. Do not invent a destination — wait for the log-viewer surface (separate ticket) to define its URL shape and mapping convention.

**Empty state contract:**
- Surrounding chrome (filter row, breadcrumbs, page title) MUST remain visible. The empty state is contained inside the table/panel container, not a full-page takeover. This matches Stitch row-1 panel 2.
- Sub-section empty (detail context, e.g., "Tool Calls (0)") is a compact inline message in `on_surface_variant` color — no icon, no button. Matches Stitch row-2 panel 5.
- `action?: { label, onClick }` renders a primary-colored tertiary text button (e.g., "Clear filters", "Try a wider time range").

**Error state contract:**
- Error icon uses `--color-error` (#ef4444) at low opacity — never raw red Tailwind classes (`text-red-400`).
- Error wording is human-shaped: "Failed to load sessions. The dashboard server returned 500." NOT raw error.message dumps. The component accepts a `message?: string` override; if absent, renders a generic-but-context-appropriate fallback.
- `retry: () => void` wires to the consumer's `useQuery` `refetch` function — primary gradient button per rulebook §5.
- `detailsHref?: string` opens an error-details surface (initially: log viewer URL or trace ID search). Optional; if absent, no secondary link rendered.
- **No raw stack traces, no `error.message` ternaries.** Consumers convert their error object to a human message before passing.

**Keyboard:** all action elements (`<button>` for retry, `<a>` for details) are keyboard-accessible by default. Loading skeletons are not focusable (purely structural). Empty/error icons are decorative (`aria-hidden="true"`).

**ARIA:** `<LoadingState />` includes `role="status"` and `aria-live="polite"` so screen readers announce loading. `<ErrorState />` includes `role="alert"` so screen readers escalate errors.

---

## 6. Roundness & Spacing

- **Roundness:** All interactive elements adhere to the `xl` (1.5rem) or `lg` (1rem) scale. Sharp corners break the "Soft Minimalism" vibe and are forbidden.
- **Spacing scale:** 8px-based (the Stitch project's `spacingScale: 2` indicates a doubled rhythm). Common increments: `spacing-1` (4px), `spacing-2` (8px), `spacing-3` (12px), `spacing-4` (16px), `spacing-5` (20px), `spacing-6` (24px), `spacing-8` (32px), `spacing-12` (48px), `spacing-16` (64px).

---

## 7. Do's and Don'ts

### Do

- **Use Asymmetric Whitespace:** Give headlines 2x more bottom margin than the body text below them to create an editorial feel.
- **Layer with Intent:** Ensure every nested container is at least one "tier" higher or lower than its parent (`surface_container` → `surface_container_high`).
- **Embrace the Dark:** Use the `background` (#0c0e11) to let the screen "breathe." High-end design is defined by what you leave out.

### Don't

- **No Pure White:** Never use `#ffffff`. All "white" text must be `on_surface` (#e8e8ec) to prevent eye strain in dark mode.
- **No Sharp Corners:** Every interactive element must adhere to the `xl` (1.5rem) or `lg` (1rem) roundedness scale.
- **No Heavy Borders:** If you find yourself reaching for a 1px solid border to solve a layout issue, use a spacing increment or a tonal background shift instead.

---

## 8. Extension policy

This rulebook grows as we discover patterns the existing rules don't cover. The growth path:

1. A surface needs a pattern not in this document (e.g., the Observability Dashboard needs a trace widget the Cloud Console doesn't).
2. The pattern is proposed to Vincent — by Claude during a Stitch session, by a contributor in a brainstorm, or by Vincent himself.
3. If accepted, the pattern is added to this document with a section identifying which surfaces consume it. **One implementation, in `@senara-solutions/ui` if shared, otherwise in the surface's local components.**
4. The pattern is now a rule. PRs that introduce the pattern reference the relevant section here.

The rulebook never splits. We never have "the Dashboard's version" of a button. If a button needs a variant, the variant is added to this document and offered to all surfaces.

## 9. Implementation surface

The technical embodiment of this rulebook lives at:

- **`mika/packages/ui/`** — `@senara-solutions/ui` shared component library. Tokens, primitives, components defined here are consumed by all three surfaces.
- **`mika/dashboard/`** — Observability Dashboard. Consumes `@senara-solutions/ui`, applies surface-specific page layouts.
- **`mika-cloud/`** — Cloud Console (frontend lives in `mika-cloud`, gateway code in `mika/crates/mika-gateway`). Consumes `@senara-solutions/ui`.
- **Landing Page** — TBD location; will consume `@senara-solutions/ui` during its reconciliation pass.

When in doubt about whether a token, primitive, or component should live in `@senara-solutions/ui` versus a surface's local components: **if more than one surface needs it, it goes in `@senara-solutions/ui`.** The default is shared.
