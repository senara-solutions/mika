# mika#1800 — Injection verification (theme.css)

Per plan § Injection verification and `feedback_verify_pipeline_passes_without_the_fix`
memory rule. Two inversions to prove the fix works.

## 1. Backward-compat alias fires

**Setup:** temporarily comment out `--color-accent: var(--color-primary);` in
`packages/ui/src/theme.css` (the aliases block near the bottom of the `@theme`
declaration).

**Test:** render any consumer that references `--color-accent` (or a
`bg-accent`/`text-accent` Tailwind utility resolving through it) — the
alias-comment-out should cause the token to fall back to whatever inline
default the consumer provides, or resolve as `initial`/empty.

**Expected:** the token resolves empty/initial when the alias is removed,
proving the alias line is what preserves legacy consumers under the new
canonical set. Restore.

**Static-file version (used by the smoke test):** the parsed `theme.css`
string MUST contain `--color-accent: var(--color-primary)`. Removing it
would break any lingering legacy consumer we missed in the grep sweep.

## 2. Canonical token defined

**Setup:** temporarily comment out `--color-primary: #ada3ff;` in
`packages/ui/src/theme.css`.

**Test:** render `TimeRangeFilter` — its class expressions use
`bg-[var(--color-primary,#ada3ff)]/20`. With `--color-primary` undefined,
the CSS falls back to the inline `#ada3ff` — visually indistinguishable
because the fallback matches the canonical value.

**Expected:** rendering works via fallback, but only because
TimeRangeFilter defensively wrote against the missing token. Restore
`--color-primary: #ada3ff` — now the token resolves against the theme
definition, and any future consumer that omits the fallback (per
LC.2+ policy) still renders correctly. This is the whole point of
LC.1 — foundation-first token definition.

**Static-file version (used by the smoke test):** the parsed `theme.css`
string MUST contain `--color-primary: #ada3ff`. Its absence is the state
that the LC milestone exists to eliminate.

## Automated coverage

`packages/ui/src/theme.test.ts` (added in this ticket) asserts:

- Canonical rulebook §2 tokens are present with the expected hex values
  (`--color-primary: #ada3ff`, `--color-background: #0c0e11`, all 7 surface
  levels, error triad, on-surface + outline pair).
- Backward-compat aliases exist and point at canonical tokens (not raw
  hex values — proves the alias mechanism is intact).
- The former legacy hex values (`#7c6af7`, `#0d0f12`, `#151820`,
  `#1e2130`, `#ef4444`) are absent from the theme file. This is the
  regression sensor — if a future PR reintroduces them, the test
  fails, forcing the operator to justify the reintroduction.

## Provenance

- Plan: `docs/plans/2026-08-23-002-fix-1800-theme-css-rulebook-alignment-plan.md`
- Ticket: mika#1800
- Milestone: mika#1799 (LC — Luminescent Core reconciliation)
