import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { describe, it, expect } from 'vitest'

/**
 * Static-file assertions on `packages/ui/src/theme.css`.
 *
 * Rationale: `theme.css` is consumed by Tailwind CSS v4 at build time via
 * the `@theme` block — CSS custom properties resolve at runtime through
 * whatever surface loads the file. In a vitest+jsdom environment we cannot
 * meaningfully evaluate `getComputedStyle(...)` against Tailwind's
 * compile-time expansion. Parsing the file text is the deterministic
 * substitute and is what pins the LC.1 (mika#1800) rulebook alignment.
 *
 * See `todos/1800-injection-verification.md` and
 * `docs/plans/2026-08-23-002-fix-1800-theme-css-rulebook-alignment-plan.md`
 * for the injection-verification framing.
 */

const HERE = dirname(fileURLToPath(import.meta.url))
const THEME_CSS_RAW = readFileSync(join(HERE, 'theme.css'), 'utf8')

// Strip CSS block comments (/* ... */) so the banned-hex assertion only
// fires on active declarations, not on comments that mention superseded
// legacy hex values for historical context. Presence assertions on
// canonical tokens and backward-compat aliases run against the same
// stripped form.
const THEME_CSS = THEME_CSS_RAW.replace(/\/\*[\s\S]*?\*\//g, '')

// Collapse whitespace runs so `--color-bg:  var(--color-background)  ;` and
// `--color-bg: var(--color-background);` both match the same probe.
const collapse = (s: string) => s.replace(/\s+/g, ' ').toLowerCase()
const NORMALIZED = collapse(THEME_CSS)

const canonicalTokens: Array<[name: string, hex: string, source: string]> = [
  // Surface hierarchy — rulebook §2 Full Token Reference
  ['--color-background', '#0c0e11', 'background/surface/surface_dim'],
  ['--color-surface', '#0c0e11', 'background/surface/surface_dim'],
  ['--color-surface-dim', '#0c0e11', 'background/surface/surface_dim'],
  ['--color-surface-container-lowest', '#000000', 'surface_container_lowest'],
  ['--color-surface-container-low', '#111317', 'surface_container_low'],
  ['--color-surface-container', '#171a1d', 'surface_container'],
  ['--color-surface-container-high', '#1d2024', 'surface_container_high'],
  ['--color-surface-container-highest', '#23262a', 'surface_container_highest'],
  ['--color-surface-variant', '#23262a', 'surface_variant'],
  ['--color-surface-bright', '#292c31', 'surface_bright'],
  // On-surface + outline
  ['--color-on-background', '#e8e8ec', 'on_background/on_surface'],
  ['--color-on-surface', '#e8e8ec', 'on_background/on_surface'],
  ['--color-on-surface-variant', '#aaabaf', 'on_surface_variant'],
  ['--color-outline', '#747579', 'outline'],
  ['--color-outline-variant', '#46484b', 'outline_variant'],
  // Primary + secondary + tertiary + surface_tint + inverse
  ['--color-primary', '#ada3ff', 'primary'],
  ['--color-primary-dim', '#715eeb', 'primary_dim'],
  ['--color-primary-container', '#9f93ff', 'primary_container/primary_fixed'],
  ['--color-primary-fixed', '#9f93ff', 'primary_container/primary_fixed'],
  ['--color-primary-fixed-dim', '#9182ff', 'primary_fixed_dim'],
  ['--color-secondary', '#9d8fff', 'secondary/secondary_dim'],
  ['--color-secondary-dim', '#9d8fff', 'secondary/secondary_dim'],
  ['--color-secondary-container', '#4434a0', 'secondary_container'],
  ['--color-tertiary', '#f6f9ff', 'tertiary'],
  ['--color-surface-tint', '#ada3ff', 'surface_tint'],
  ['--color-inverse-surface', '#f9f9fd', 'inverse_surface'],
  // Error triad — §2 canonical (§5.5 line 270 contradiction surfaced to Vincent
  // in the PR body as operator-owned rulebook follow-up).
  ['--color-error', '#ff6e84', 'error (§2 canonical, adopted over §5.5 #ef4444)'],
  ['--color-error-dim', '#d73357', 'error_dim'],
  ['--color-error-container', '#a70138', 'error_container'],
]

const backwardCompatAliases: Array<[legacy: string, canonical: string]> = [
  ['--color-bg', 'var(--color-background)'],
  ['--color-bg-card', 'var(--color-surface-container)'],
  ['--color-accent', 'var(--color-primary)'],
  ['--color-accent-light', 'var(--color-secondary)'],
  ['--color-heading', 'var(--color-on-surface)'],
  ['--color-muted', 'var(--color-on-surface-variant)'],
]

// Formerly-live legacy hex values — must be absent from the theme file after
// LC.1 reconciliation. Reintroducing any of them regresses the fix and forces
// the operator to justify the reintroduction (or update this list with a
// documented rationale).
const bannedLegacyHexValues: Array<[hex: string, wasFor: string]> = [
  ['#7c6af7', 'legacy accent — replaced by canonical primary #ada3ff'],
  ['#0d0f12', 'legacy bg — replaced by canonical background #0c0e11'],
  ['#151820', 'legacy bg-card — replaced by canonical surface_container #171a1d'],
  ['#1e2130', 'legacy surface-container-high — replaced by canonical #1d2024'],
  ['#e8ecf2', 'legacy heading — replaced by canonical on_surface #e8e8ec'],
  ['#a0a8b8', 'legacy muted — replaced by canonical on_surface_variant #aaabaf'],
  ['#ef4444', '§5.5 error hex — resolved to §2 canonical #ff6e84 in theme'],
]

describe('theme.css — LC.1 (mika#1800) rulebook §2 alignment', () => {
  it.each(canonicalTokens)(
    'defines canonical token %s = %s (%s)',
    (name, hex) => {
      const probe = collapse(`${name}: ${hex};`)
      expect(NORMALIZED.includes(probe)).toBe(true)
    },
  )

  it.each(backwardCompatAliases)(
    'preserves backward-compat alias %s → %s',
    (legacy, canonical) => {
      const probe = collapse(`${legacy}: ${canonical};`)
      expect(NORMALIZED.includes(probe)).toBe(true)
    },
  )

  it.each(bannedLegacyHexValues)(
    'does not reintroduce legacy hex %s (%s)',
    (hex) => {
      // Case-insensitive; the alias mechanism means legacy names now point at
      // `var(...)`, so no raw legacy hex should remain among the file's
      // active declarations (comments referencing superseded values are OK —
      // stripped via THEME_CSS pre-processing at the top of this file).
      expect(THEME_CSS.toLowerCase()).not.toContain(hex.toLowerCase())
    },
  )
})
