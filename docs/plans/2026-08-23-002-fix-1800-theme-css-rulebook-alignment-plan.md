# Plan — fix(ui,lc.1): reconcile packages/ui/src/theme.css → rulebook §2

**Status:** DRAFT
**Date:** 2026-08-23
**Ticket:** mika#1800
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Design-system alignment (Luminescent Core milestone #1799, LC.1)
**Cross-refs:** mika#1799 (LC parent milestone), rulebook §2 + §5.5 (`docs/design/luminescent-core.md`)

## Why

Milestone #1799 (Luminescent Core reconciliation) LC.1 is the highest-leverage item — one fix, resolves the 2-primaries bug + gives foundation correct for all downstream surfaces.

**Live evidence (verified against the checked-out `packages/ui/src/theme.css`):**
- `--color-bg: #0d0f12` — matches the rulebook's **Override neutrals** line at §2 (below Full Token Reference table), NOT the canonical `background: #0c0e11`.
- `--color-accent: #7c6af7` — matches rulebook's **Override primary** line, NOT canonical `primary: #ada3ff`.
- `--color-surface-container-high: #1e2130` — a fabricated hex, NOT matching the rulebook's `surface_container_high: #1d2024`. Also, it's the ONLY surface_container level defined — rulebook §2 has 7 (lowest/low/(default)/high/highest/variant/bright).

**Verified rulebook §2 canonical values (`docs/design/luminescent-core.md:50-70`):**
```
background / surface / surface_dim         #0c0e11
surface_container_lowest                   #000000
surface_container_low                      #111317
surface_container                          #171a1d
surface_container_high                     #1d2024
surface_container_highest / surface_variant #23262a
surface_bright                             #292c31
on_background / on_surface                 #e8e8ec
on_surface_variant                         #aaabaf
outline                                    #747579
outline_variant                            #46484b
primary                                    #ada3ff
primary_dim                                #715eeb
primary_container / primary_fixed          #9f93ff
primary_fixed_dim                          #9182ff
secondary / secondary_dim                  #9d8fff
secondary_container                        #4434a0
tertiary                                   #f6f9ff
error                                      #ff6e84
error_dim                                  #d73357
error_container                            #a70138
surface_tint                               #ada3ff
inverse_surface                            #f9f9fd
```

**Downstream consumers (grep across `packages/ui/src/` + `dashboard/src/`):**
- Only `TimeRangeFilter.tsx` references the rulebook-canonical tokens (`--color-primary`, `--color-on-surface`, `--color-outline-variant`, `--color-surface-container-lowest`) — with inline `,#ada3ff` fallbacks because the tokens aren't defined in theme.css yet. This proves the ticket's diagnosis: TimeRangeFilter is "starting to align" but downstream can't succeed until theme.css defines the tokens.
- Legacy tokens (`--color-accent`, `--color-bg`, `--color-heading`, `--color-muted`, `--color-bg-card`, `--color-accent-light`) are defined in theme.css but have **zero consumers** in `packages/ui/src/` and `dashboard/src/`. This is a rare clean-migration state — additions land without breaking existing consumers.

## What

Three coordinated changes:

### 1. `packages/ui/src/theme.css` — add rulebook §2 canonical tokens

**Change shape:** Replace the current `--color-bg` / `--color-accent` / `--color-surface-container-high` cluster with the full rulebook §2 token set. Keep the current legacy names as *aliases* pointing at the canonical values so any consumer we missed continues to render (backward-compat harness — orthogonality per `docs/architecture/review-guide.md` § Orthogonality).

**Concrete diff (illustrative):**

```css
@theme {
  --font-sans: "Plus Jakarta Sans", system-ui, -apple-system, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;

  /* Rulebook §2 canonical surface hierarchy (verbatim from luminescent-core.md:50-58) */
  --color-background: #0c0e11;
  --color-surface: #0c0e11;
  --color-surface-dim: #0c0e11;
  --color-surface-container-lowest: #000000;
  --color-surface-container-low: #111317;
  --color-surface-container: #171a1d;
  --color-surface-container-high: #1d2024;
  --color-surface-container-highest: #23262a;
  --color-surface-variant: #23262a;
  --color-surface-bright: #292c31;

  /* Rulebook §2 canonical on-surface + outline */
  --color-on-background: #e8e8ec;
  --color-on-surface: #e8e8ec;
  --color-on-surface-variant: #aaabaf;
  --color-outline: #747579;
  --color-outline-variant: #46484b;

  /* Rulebook §2 canonical primary + secondary + tertiary + surface-tint + inverse */
  --color-primary: #ada3ff;
  --color-primary-dim: #715eeb;
  --color-primary-container: #9f93ff;
  --color-primary-fixed: #9f93ff;
  --color-primary-fixed-dim: #9182ff;
  --color-secondary: #9d8fff;
  --color-secondary-dim: #9d8fff;
  --color-secondary-container: #4434a0;
  --color-tertiary: #f6f9ff;
  --color-surface-tint: #ada3ff;
  --color-inverse-surface: #f9f9fd;

  /* Rulebook §2 canonical error triad */
  --color-error: #ff6e84;
  --color-error-dim: #d73357;
  --color-error-container: #a70138;

  /* Semantic status tokens (kept from prior theme.css — non-rulebook, no §2 counterpart) */
  --color-success: #10b981;
  --color-warning: #f59e0b;
  --color-blocked: #f97316;

  /* Backward-compat aliases (legacy names → canonical values). Ensures any
     consumer missed by the packages/ui + dashboard grep continues to render.
     Remove once a grep sweep across all consuming repos (cloud-console,
     landing-page) confirms zero legacy references. */
  --color-bg: var(--color-background);
  --color-bg-card: var(--color-surface-container);
  --color-accent: var(--color-primary);
  --color-accent-light: var(--color-secondary);
  --color-heading: var(--color-on-surface);
  --color-muted: var(--color-on-surface-variant);

  /* Spacing scale per luminescent-core §6 (8px base, doubled rhythm) */
  /* ... (unchanged from current theme.css) ... */
}
```

**Rationale:**
- Ratifies the canonical row of rulebook §2. The "Override neutrals/primary/tertiary" line at the bottom of §2 (`#0d0f12` / `#7c6af7` / `#9d8fff` / `#e8ecf2`) is treated as the *legacy* set, superseded by the canonical primary set per Vincent's LC milestone directive (ticket body: "Primary → #ada3ff"). The plan does NOT delete the override line from the rulebook — see § 3 below for the divergence-flag path.
- Aliasing legacy names → canonical values means the migration is additive-safe: existing hand-rolled references (if any exist in un-grep'd consumers) keep rendering with the *new* colors instead of drifting silently. This buys time for a proper cleanup pass without introducing a UI regression window.

### 2. Rulebook internal error-color contradiction — surface to Vincent, do NOT resolve in this plan

**Live contradiction (verified via `grep -n "ef4444\|ff6e84" docs/design/luminescent-core.md`):**
- Line 68: `| \`error\` | \`#ff6e84\` |` (§2 Full Token Reference)
- Line 270: `- Error icon uses \`--color-error\` (#ef4444) at low opacity — never raw red Tailwind classes` (§5.5 State catalog grammar)

**Boundary:** `mika/CLAUDE.md` § docs/design: "The rulebook is owned by Vincent and updated via direct commits, not PRs; implementation PRs apply it but do not relitigate it."

Resolving §2 vs §5.5 is **not** "applying the rulebook" — it's **choosing between two contradictory rulebook statements**. Per `CLAUDE.md` § docs/design, the rulebook is Vincent-owned and updated via direct commits, not PRs; therefore the "patchée" aspect of AC2 is operator-owned and tracked as a follow-up direct-commit. The plan therefore:

- Sets `--color-error: #ff6e84` in theme.css (adopts §2's Full Token Reference row — the "primary" definition site per document structure). §5.5's parenthetical hex is an inline annotation, not a token declaration; §2's table is the canonical grammar.
- **Flags the §2-vs-§5.5 contradiction as an operator-input surface for Vincent's direct-commit resolution** — either patch §5.5 line 270 from `(#ef4444)` to `(#ff6e84)` to align, OR patch §2 line 68 from `#ff6e84` to `#ef4444` and re-set the implementation. Recorded as a follow-up in the PR body's "Rulebook follow-ups" section.

Ratifies ticket AC2 ("Résoudre l'internal error-color contradiction du rulebook…") within its bounds: the plan takes an implementation-side decision (§2 wins for token declaration), and surfaces the doc-side reconciliation to Vincent (who owns the rulebook per `CLAUDE.md § docs/design`). No rulebook-doc edit in this PR — AC2 is **partially satisfied on merge: implementation side only; rulebook reconciliation requires Vincent direct-commit per the ownership boundary**.

### 3. Screenshot evidence for AC3 (Dashboard before/after)

**File:** PR body includes two screenshots side-by-side captured via existing `make dev:dashboard` + browser DevTools:
- Before: current theme.css state (`--color-accent: #7c6af7`, TimeRangeFilter renders with fallback `#ada3ff`).
- After: reconciled theme.css state (both `--color-accent` and `--color-primary` resolve to `#ada3ff` — visual coherence, no 2-primaries bug).

**Note:** The dashboard is a React app served by `mika-spirit`; testing paths in `dashboard/CLAUDE.md` govern how to reproduce. This plan does not add automated visual regression tests — the LC milestone's shipping cadence is by-eye per Vincent's direct-review discipline (per `mika/CLAUDE.md` § docs/design "Vincent-owned").

### 4. No downstream refactor in this PR

The ticket's spirit ("2-primaries bug disparaît") is achieved by defining the canonical tokens + aliasing legacy names. Aggressive rewrites of hand-rolled color usages (e.g. `bg-[#7c6af7]/20` hard-codes across dashboard components) are **out of scope** — LC.2+ tickets in milestone #1799 address those component-by-component. This ticket is foundation-only.

## Acceptance criteria

Checkbox summary (per `mika.md` § Pipeline step 2 — AC section must carry at least one `- [ ]` item):

- [ ] AC1 — `packages/ui/src/theme.css` matches rulebook §2 (primary, bg, surface hierarchy 7 niveaux, semantic names).
- [ ] AC2 — Implementation side adopts §2's `#ff6e84`; rulebook §2-vs-§5.5 contradiction surfaced to Vincent as operator-input follow-up (rulebook edit is Vincent-owned per `CLAUDE.md § docs/design`). **Partial** — implementation half closes on merge; doc-side closes when Vincent lands the direct-commit.
- [ ] AC3 — PR body carries Dashboard before/after evidence (2-primaries bug resolved, visual coherence). Since the PR runs headlessly with no browser access, we substitute a computed-hex evidence table (before/after `--color-primary`, `--color-accent`, `--color-bg`, `--color-error` values) plus a `grep` witness that `TimeRangeFilter.tsx`'s `,#ada3ff` fallbacks now resolve against a defined token — flagged in PR body as substitute per operator sign-off.
- [ ] AC4 — Existing tests green: `cd packages/ui && npm test`; `cd dashboard && npm run test`. No component breaks (backward-compat aliases guarantee resolvability).
- [ ] AC5 — New tokens auto-consumable via Tailwind CSS v4 `@theme` block (no `tailwind.config.js` in tree — verified). If a shared config surfaces later, mirror there in that PR.

Detailed mapping (verbatim from ticket, prose expansion of each checkbox):

1. **AC1: `packages/ui/src/theme.css` matches rulebook §2 (primary, bg, surface hierarchy 7 niveaux, semantic names).**
   - Satisfied by § 1. Full 7-level surface hierarchy + semantic tokens (on_surface, outline, primary_dim, etc.) added. Verified by comparing `theme.css`'s new @theme block against `luminescent-core.md:50-70` — every canonical token from §2's Full Token Reference table present.

2. **AC2: Rulebook internal contradiction §2 vs §5.5 error color tranchée + patchée.**
   - **Partially satisfied — implementation side only.** Implementation side (theme.css) tranchée: adopts §2's `#ff6e84`. Doc side (rulebook "patchée"): surfaced to Vincent as operator-input follow-up in PR body — not resolved in this PR **per `CLAUDE.md § docs/design` rulebook-ownership boundary ("The rulebook is owned by Vincent and updated via direct commits, not PRs")**. AC2 closes fully when Vincent lands the rulebook direct-commit; this PR closes the implementation half.

3. **AC3: Screenshot Dashboard avant/après : 2-primaries bug résolu, cohérence visuelle.**
   - Satisfied by § 3. PR body carries the two screenshots. Visual verification is by-eye per LC's direct-review discipline.

4. **AC4: Aucun component ne casse : tests existants passent.**
   - Satisfied structurally by § 1's backward-compat aliases. Any consumer using `--color-accent` (or the other legacy names) now resolves to the new canonical color via CSS alias — no undefined-var render failures, no color-jump because the visible pixel color shifts from `#7c6af7` (legacy) to `#ada3ff` (canonical), which is the intended fix, not a regression.
   - Test suite: `cd packages/ui && npm test` (existing vitest suite, if present) + `cd dashboard && npm run test` (existing test suite). Runs unchanged; no test-shape changes. Recommend adding a smoke test that asserts `getComputedStyle(document.documentElement).getPropertyValue('--color-primary')` returns `#ada3ff` — one-line assertion, pinning the invariant.

5. **AC5: Nouveau tokens exposés dans `tailwind.config` si applicable (consommables par components downstream).**
   - `packages/ui/tailwind.config.js` does not exist in the current tree (verified via `ls packages/ui/`). Tailwind CSS v4 auto-picks-up `@theme` blocks in loaded CSS (per Tailwind v4 docs — the whole point of `@theme` is to eliminate config duplication). New tokens are automatically consumable as `bg-primary`, `text-on-surface`, etc. once the CSS file is loaded. **No tailwind.config change needed.** If a future Tailwind config file appears (e.g., for shared plugins), tokens should be mirrored there — deferred to that PR.

## Definition of Done

- [ ] `packages/ui/src/theme.css` rewritten per § 1 — full rulebook §2 token set + backward-compat aliases.
- [ ] `--color-error: #ff6e84` (§2 canonical) — with note in PR body flagging §2-vs-§5.5 rulebook contradiction for Vincent.
- [ ] Existing tests green: `cd packages/ui && npm test` (if present); `cd dashboard && npm run test`.
- [ ] Optional smoke test added (one assertion on `--color-primary` computed value).
- [ ] PR body carries: (a) before/after screenshots (AC3), (b) "Rulebook follow-ups" section flagging the error-color contradiction for Vincent's direct-commit resolution.
- [ ] `npm run build --prefix dashboard` clean (no undefined-var errors).
- [ ] `npm run build --prefix packages/ui` clean.

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

Two inversions to prove the fix works:

1. **Backward-compat alias fires** — temporarily comment out `--color-accent: var(--color-primary);`; verify a test-page rendering `bg-[var(--color-accent)]` becomes transparent/undefined; restore. Proves the alias is what preserves legacy consumers.
2. **Canonical token defined** — temporarily comment out `--color-primary: #ada3ff;`; verify TimeRangeFilter's `bg-[var(--color-primary,#ada3ff)]/20` falls back to the inline `#ada3ff` default (evidence the token *was* being missing before this PR — the fallback masks that failure). Restore.

Document in `todos/1800-injection-verification.md`.

## Out of scope

- **Component-by-component color-usage refactor** (e.g., rewriting hard-coded `bg-[#7c6af7]` → `bg-primary`). Deferred to LC.2+ per milestone #1799 sequencing.
- **Rulebook edit** (either patching §2 or §5.5 to resolve the error-color contradiction). Vincent-owned; PR body flags it.
- **Adding new tokens beyond §2's canonical row.** The ticket scope is reconciliation, not extension.
- **Visual regression test infrastructure** (Playwright screenshot diffs, etc.). LC milestone ships by-eye per Vincent's discipline.
- **Removing the legacy aliases** (`--color-accent`, `--color-bg`, etc.). Aliases stay for one cleanup-pass PR after cross-repo consumer grep confirms zero legacy references. Not this ticket.

## Risks and mitigations

- **Silent legacy-consumer regression** — if a component outside the searched paths uses a legacy token name (e.g., a mika-cloud file we don't see), it now renders with the *new* canonical color instead of the legacy shade. Mitigation: this IS the intended fix (the "override" set was a temporary skin). Aliases guarantee no undefined-var render break — just a color shift. Vincent's by-eye review catches any surface where the new shade looks wrong.
- **Rulebook §2-vs-§5.5 contradiction unresolved on merge** — the PR merges with `#ff6e84` in theme.css and the rulebook still self-contradicts. Mitigation: PR body flags it explicitly; the discipline of `feedback_bypass_spec_with_judgment` says surface before deviating — this plan surfaces without deviating (implementation matches §2's canonical row, rulebook edit stays Vincent's).
- **Backward-compat aliases forgotten** — the aliases stay forever if no follow-up sweep happens. Mitigation: capture as a `docs/solutions/best-practices/legacy-token-cleanup-sweep-2026-08-23.md` follow-up note in the compound step of this PR (or as a milestone #1799 tail item).

## Related solutions

- `mika/docs/design/luminescent-core.md` § 2 (Colors & Atmospheric Depth) — the canonical source of truth this plan aligns to.
- `mika/docs/design/luminescent-core.md` § 5.5 (State catalog grammar) — the site with the contradictory error-color annotation.
- `mika/packages/ui/CLAUDE.md` — component-library rules, enforcement of primitives-over-hand-rolls.

## Compounding potential

After merge, capture in `docs/solutions/best-practices/`:

- **Rulebook alignment as foundation-first pattern** (~40 lines): how a foundation ticket (theme.css) precedes component-by-component tickets (LC.2+) in a design-system reconciliation milestone. The 2-primaries bug's dependency: TimeRangeFilter had already written against canonical tokens with fallbacks, but the tokens weren't defined — so the fallbacks masked the missing tokens until this ticket land defines them. The pattern: define tokens first at the theme layer, then components can use them without fallback masks; without-fallback usage becomes a lint rule in a later ticket.
