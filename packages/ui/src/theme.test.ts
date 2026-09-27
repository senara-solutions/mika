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
// `--color-bg: var(--color-background);` both match the same shape.
const collapse = (s: string) => s.replace(/\s+/g, ' ').toLowerCase()

/**
 * Parse `--<name>: <value>;` declarations into a `Map<name, value>` with
 * last-wins semantics — mirrors CSS cascade so a future PR that accidentally
 * duplicates a token declaration is caught (the earlier declaration would be
 * shadowed and pass a simple `includes()` sensor, letting silent drift through).
 * Values are collapsed + lowercased for match stability.
 */
const declarationMap: Map<string, string> = (() => {
  const map = new Map<string, string>()
  const re = /(--[a-z0-9-]+)\s*:\s*([^;]+);/gi
  let m: RegExpExecArray | null
  while ((m = re.exec(THEME_CSS)) !== null) {
    map.set(m[1].toLowerCase(), collapse(m[2]).trim())
  }
  return map
})()

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

/**
 * Every rule whose selector begins with `.mika-cta-gradient`, as
 * `[selector, body]` pairs — collapsed and lowercased like the token values
 * above. The regex deliberately admits a selector suffix (`:not(:disabled):hover`)
 * so the hover rule is inside the population: a hover that drifted to a hex
 * literal would be just as silent a §2 desync as the rest state doing it.
 */
const ctaGradientRules: Array<[selector: string, body: string]> = (() => {
  const rules: Array<[string, string]> = []
  const re = /(\.mika-cta-gradient[^{}]*)\{([^}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(THEME_CSS)) !== null) {
    rules.push([collapse(m[1]).trim(), collapse(m[2]).trim()])
  }
  return rules
})()

describe('theme.css — LC.1 (mika#1800) rulebook §2 alignment', () => {
  it.each(canonicalTokens)(
    'defines canonical token %s = %s (%s)',
    (name, hex) => {
      // `.toBe()` on the final value (not `.includes()` on a substring)
      // so a failure prints "expected <hex>, received <actual>" pointing
      // straight at the drift. Also catches a *duplicate* declaration with
      // a different hex — `declarationMap` uses last-wins, mirroring CSS
      // cascade; the sensor's presence in the map is uniqueness + value.
      expect(declarationMap.get(name.toLowerCase())).toBe(hex.toLowerCase())
    },
  )

  it.each(backwardCompatAliases)(
    'preserves backward-compat alias %s → %s',
    (legacy, canonical) => {
      expect(declarationMap.get(legacy.toLowerCase())).toBe(
        collapse(canonical).trim(),
      )
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

/**
 * LC.2 (mika#1801) — the CTA texture.
 *
 * Rulebook §2 "Signature Textures" (`luminescent-core.md:44`) and §5 "Buttons"
 * (l.105) both prescribe a `primary` -> `primary_dim` linear gradient at 135deg
 * for main CTAs. Until LC.2 **no surface applied it**: every CTA in the repo was
 * a flat `bg-accent`. The texture is declared here, once, rather than in
 * `<Button>`'s TSX, for three reasons the plan states (D2):
 *
 *   1. `theme.css` is the one file all three surfaces already import — including
 *      `site/`, which imports nothing else from the package — so the texture
 *      reaches a CTA even where the primitive does not.
 *   2. Tailwind v4 has no utility for a 135deg gradient between two arbitrary CSS
 *      variables, so the alternative is a verbose arbitrary value repeated at
 *      every callsite.
 *   3. This file is already pinned by static assertion, so the texture becomes
 *      pinnable by the same mechanism, beside the values it composes. Written in
 *      TSX it would be invisible to this guard.
 *
 * What these assertions protect is not the pixels — it is that the texture keeps
 * **tracking §2 through the tokens**. A hex literal here renders identically
 * today and stops moving the day the rulebook moves, which is the exact defect
 * `scripts/check-landing-tokens.sh` exists to catch one surface over.
 */
describe('theme.css — LC.2 (mika#1801) rulebook §2/§5 CTA texture', () => {
  it('declares the .mika-cta-gradient rest state', () => {
    const selectors = ctaGradientRules.map(([sel]) => sel)
    expect(selectors).toContain('.mika-cta-gradient')
  })

  it('declares a hover state scoped away from :disabled', () => {
    // `:hover` fires on a disabled <button> in every engine, so an unscoped
    // hover would animate a button that cannot be pressed. `:not(:disabled)` is
    // vacuously true on the <a> branch of <Button>, so one selector serves both.
    const hover = ctaGradientRules.find(([sel]) => sel.includes(':hover'))
    expect(hover).toBeDefined()
    expect(hover?.[0]).toContain(':not(:disabled)')
  })

  it('renders the rest state as a 135deg gradient from primary to primary_dim', () => {
    const rest = ctaGradientRules.find(([sel]) => sel === '.mika-cta-gradient')
    const body = rest?.[1] ?? ''

    expect(body).toContain('linear-gradient')
    // §2: "at a 135-degree angle".
    expect(body).toContain('135deg')

    // §2/§5: `primary` FIRST, then `primary_dim`. The closing paren in
    // `var(--color-primary)` is what makes this unambiguous — without it the
    // needle would also match inside `var(--color-primary-dim)`.
    const from = body.indexOf('var(--color-primary)')
    const to = body.indexOf('var(--color-primary-dim)')
    expect(from).toBeGreaterThanOrEqual(0)
    expect(to).toBeGreaterThanOrEqual(0)
    expect(from).toBeLessThan(to)
  })

  it.each([0, 1])('carries no colour literal in rule #%i', (i) => {
    // The load-bearing assertion. `bannedLegacyHexValues` above is a denylist,
    // which is right for the token table whose whole content is known values;
    // it is green on a *new* purple written in good faith. Here the rule is
    // shape: the texture may only reference tokens.
    const [selector, body] = ctaGradientRules[i] ?? ['<missing>', '']
    expect(ctaGradientRules.length).toBeGreaterThan(i)
    expect(body, `${selector} must compose tokens, never a literal`).not.toMatch(
      /#[0-9a-f]{3,8}\b/,
    )
    expect(body).not.toMatch(/\brgba?\s*\(/)
  })
})
