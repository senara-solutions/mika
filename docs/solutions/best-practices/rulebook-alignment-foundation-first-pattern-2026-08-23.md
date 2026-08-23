---
module: packages/ui
tags: [design-system, luminescent-core, tokens, tailwind-v4, reconciliation, foundation-first]
problem_type: process
category: best-practices
date: 2026-08-23
---

# Design-system alignment — foundation-first token pattern

## Problem

A design-system reconciliation milestone (Luminescent Core / mika#1799) contains
one item that resolves an observable bug ("2-primaries bug" on Dashboard: two
purple accents render side-by-side because the theme file's `--color-accent`
uses one hex and downstream components use another) and a dozen component-level
follow-ups that use hard-coded hex.

The tempting shape is "sweep everything in one PR" — rewrite theme.css AND
migrate every hand-rolled `bg-[#7c6af7]/20` in one merge. That shape has three
failure modes:

1. **Foundation-and-consumer coupling.** Every consumer migration depends on
   the token being defined. A monolithic PR forces a specific merge order
   that's easy to violate mid-review.
2. **Silent fallback masking.** Components that pre-write against
   rulebook-canonical tokens with inline defaults (e.g.
   `bg-[var(--color-primary,#ada3ff)]`) succeed today because the fallback
   fires — but the ONLY reason they render correctly is the fallback. Delete
   the fallback and nothing changes visibly, hiding a foundation gap. This is
   observed in mika#1800's live evidence.
3. **Review bandwidth exhaustion.** A monolithic dozen-file PR gets rubber-
   stamped or bounced for redo. Foundation-first splits the surface: one
   small foundation PR that a reviewer can hold in their head, then component
   PRs that reference the foundation.

## Solution

Sequence design-system reconciliation as **LC.1 = foundation, LC.2+ = consumers**:

### LC.1 (foundation)

- **Change theme.css only.** Add the rulebook's canonical token set — every
  hex, every semantic name, in one block.
- **Preserve legacy names as aliases.** Any consumer using
  `--color-accent` (legacy) now resolves to
  `var(--color-primary)` (canonical). Colors shift to the canonical hex
  (the intended fix), but nothing renders undefined.
- **Add a static-file smoke sensor.** Parse the theme.css declarations into a
  Map with last-wins semantics and assert every canonical token + alias +
  banned-legacy-hex. See `packages/ui/src/theme.test.ts` for the shape. This
  is the regression sensor that pins the foundation.
- **Do not touch consumers.** Component-level hand-coded hex (e.g. Recharts
  props that take literal JS strings) stays as-is for LC.2+.
- **Surface (do not resolve) rulebook contradictions.** If the rulebook
  contains internal contradictions (e.g. mika/docs/design/luminescent-core.md
  §2 and §5.5 both name the error color but disagree on hex), the
  implementation picks one, adopts it, and files the doc-side reconciliation
  as an operator-owned follow-up. Rulebook edits are Vincent-owned direct
  commits per `mika/CLAUDE.md § docs/design`; a PR that edits the rulebook
  violates the ownership boundary.

### LC.2+ (consumers)

- **One PR per consumer surface.** CostTrendChart hex migration, LlmCalls
  page cost line color, etc. — each ticket its own scope.
- **Migrate hand-coded hex to token references.** For CSS: `var(--color-*, #fallback)`.
  For JS (Recharts, D3, canvas): pass the token through props or resolve via
  `getComputedStyle(document.documentElement).getPropertyValue(...)`.
- **Sensor extension optional.** The foundation sensor pins the theme file;
  consumer PRs may add per-component sensors if the migration surface is
  large.

## Key decisions

- **Aliases stay until cross-repo grep confirms zero legacy references.**
  Removing `--color-accent: var(--color-primary)` too early breaks any
  consumer we missed in the initial packages/ui + dashboard grep (e.g. a
  future landing-page or cloud-console). Aliases are a cheap harness with
  one long-term cost (lingering legacy names in the theme file); a
  scheduled cleanup pass removes them once cross-repo consumers are
  audited.
- **Test file placement.** `packages/ui/src/theme.test.ts` is at the src
  root, not under `components/`. Vitest's default `include: [**/*.test.ts]`
  picks it up; the placement signals "this is a file-shape test, not a
  component test."
- **Ban list, not allow list.** The banned-hex sensor lists formerly-live
  legacy values (`#7c6af7`, `#0d0f12`, …). A future PR that reintroduces
  any of them fails the sensor, forcing a rationale. An allow-list of
  canonical hexes would explode combinatorially and reject any semantic
  color the design system adds.
- **Comment-strip before banned-hex check.** The theme file's comments
  legitimately mention superseded values ("supersedes #7c6af7 …") for
  historical context. The sensor pre-processes with a CSS-comment regex
  so only active declarations count.
- **Screenshot substitute for headless pipelines.** When the pipeline has
  no browser, an AC that says "screenshot before/after" is honored by a
  computed-hex evidence table in the PR body (before/after token values
  + a `grep` witness that fallback-mask consumers now resolve through a
  defined token). Flag the substitution in the PR body so the operator's
  by-eye review still triggers.

## Anti-patterns to avoid

- **Monolithic theme + consumer sweep.** See failure modes above.
- **Deleting fallbacks in the foundation PR.** Fallbacks are load-bearing
  until the foundation lands. Delete them in the consumer PR that
  demonstrably renders through the canonical token.
- **Editing the rulebook to match a contradictory implementation.** The
  rulebook is the source of truth; implementations follow it. Surface
  contradictions to the operator; never adjudicate them mid-implementation
  PR.
- **`.includes()` on theme.css as the sensor primitive.** Duplicate
  declarations pass a substring check but CSS renders the last one. Parse
  into a Map with last-wins semantics.

## Anchor case

- Ticket: `mika#1800`
- Milestone: `mika#1799` (Luminescent Core reconciliation)
- Plan: `docs/plans/2026-08-23-002-fix-1800-theme-css-rulebook-alignment-plan.md`
- Sensor: `packages/ui/src/theme.test.ts`
- Injection verification: `todos/1800-injection-verification.md`
