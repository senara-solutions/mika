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

### Input Fields

- Background: `surface_container_lowest`.
- Border: `outline_variant` at 10% opacity.
- Focus State: Border transitions to `primary` with a 4px outer `primary_dim` blur at 10% opacity.

### Additional Component: The "Pulse" Console

For AI platforms, we introduce the **Console Component**: A `surface_container_highest` block using **JetBrains Mono** text. It features no borders but utilizes a "Purple Glow" (a `primary` radial gradient at 5% opacity) in the top-right corner to suggest the agent is "thinking."

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
