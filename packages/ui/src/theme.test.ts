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
 * Every rule whose selector mentions a `.mika-` class, as `[selector, body]`
 * pairs — collapsed and lowercased like the token values above.
 *
 * The regex admits a selector suffix (`:not(:disabled):hover`, `[aria-disabled]`)
 * and a comma-separated group, so hover and disabled rules are inside the
 * population: one of them drifting to a hex literal would be just as silent a §2
 * desync as the rest state doing it.
 */
const mikaRules: Array<[selector: string, body: string]> = (() => {
  const rules: Array<[string, string]> = []
  const re = /([^{}]*\.mika-[^{}]*)\{([^{}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(THEME_CSS)) !== null) {
    rules.push([collapse(m[1]).trim(), collapse(m[2]).trim()])
  }
  return rules
})()

const ruleFor = (selector: string): string =>
  mikaRules.find(([sel]) => sel === selector)?.[1] ?? ''

const hasRule = (selector: string): boolean =>
  mikaRules.some(([sel]) => sel === selector)

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
    expect(hasRule('.mika-cta-gradient')).toBe(true)
  })

  it('declares a hover state scoped away from :disabled', () => {
    // `:hover` fires on a disabled <button> in every engine, so an unscoped
    // hover would animate a button that cannot be pressed. `:not(:disabled)` is
    // vacuously true on the <a> branch of <Button>, so one selector serves both.
    expect(hasRule('.mika-cta-gradient:not(:disabled):hover')).toBe(true)
  })

  it('renders the rest state as a 135deg gradient from primary to primary_dim', () => {
    const body = ruleFor('.mika-cta-gradient')

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
})

/**
 * LC.2 (mika#1801) — `<Button>` / `<Spinner>` presentation.
 *
 * These live in CSS rather than in Tailwind utilities because **Tailwind does not
 * scan `packages/ui`** — measured on this tree, with the evidence table in
 * `theme.css`'s own block comment. A utility written inside this package is
 * simply never generated, so `<Button>` built on utilities would ship with no
 * padding, no radius and no focus ring. The rules below are therefore not a
 * stylistic preference; they are the only form that reaches a consumer.
 *
 * That also makes them the right place to assert rulebook §5 conformance: the
 * component test pins the class *mapping* (which variant emits which class), and
 * these pin what those classes *mean*.
 */
describe('theme.css — LC.2 (mika#1801) rulebook §5 button presentation', () => {
  it.each([
    ['--radius-mika-xl', '1.5rem'],
    ['--radius-mika-lg', '1rem'],
  ])('declares the §6 roundedness token %s = %s', (name, value) => {
    expect(declarationMap.get(name)).toBe(value)
  })

  it('does not redefine Tailwind’s own radius scale', () => {
    // The §6 names (`xl` = 1.5rem, `lg` = 1rem) collide with Tailwind v4's
    // (`xl` = 0.75rem, `lg` = 0.5rem) at different values. Declaring the
    // rulebook's values under Tailwind's names would silently move every
    // `rounded-xl` / `rounded-lg` already written across the dashboard and the
    // landing. The prefixed tokens above exist precisely to avoid that.
    expect(declarationMap.has('--radius-xl')).toBe(false)
    expect(declarationMap.has('--radius-lg')).toBe(false)
  })

  it('gives every button the §6/§7 roundedness, from the token', () => {
    // §7 Don't: "Every interactive element must adhere to the `xl` (1.5rem) or
    // `lg` (1rem) roundedness scale." On the base rule, so no variant can miss it.
    expect(ruleFor('.mika-btn')).toContain('border-radius: var(--radius-mika-xl)')
  })

  it('gives every button a visible focus indicator', () => {
    expect(ruleFor('.mika-btn:focus-visible')).toContain('outline')
  })

  it('styles the disabled state on both rendered elements', () => {
    // <a> has no `:disabled`, so the selector group must carry the aria form too
    // or a future disabled-link affordance loses the styling in silence.
    const disabled = mikaRules.find(([sel]) => sel.includes(':disabled,'))
    expect(disabled?.[0]).toContain('[aria-disabled=')
    expect(disabled?.[1]).toContain('cursor: not-allowed')
  })

  it.each(['sm', 'md', 'lg'])('declares size %s', (size) => {
    expect(hasRule(`.mika-btn--${size}`)).toBe(true)
  })

  it('primary carries no border, per §5', () => {
    // §5: "Primary: Gradient fill ..., `xl` (1.5rem) roundedness. No border."
    expect(ruleFor('.mika-btn--primary')).toContain('border: 0')
  })

  it('primary text is on_surface, never pure white, per §7', () => {
    // §7 Don't: "No Pure White: Never use `#ffffff`. All 'white' text must be
    // `on_surface` (#e8e8ec)." The five migrated CTAs all carried `text-white`.
    expect(ruleFor('.mika-btn--primary')).toContain('color: var(--color-on-surface)')
  })

  it('secondary is a ghost with the outline_variant border at 20%, per §5', () => {
    const body = ruleFor('.mika-btn--secondary')
    expect(body).toContain('background-color: transparent')
    expect(body).toContain('var(--color-outline-variant)')
    expect(body).toContain('20%')
  })

  it('secondary hovers to secondary_container, per §5', () => {
    expect(ruleFor('.mika-btn--secondary:not(:disabled):hover')).toContain(
      'background-color: var(--color-secondary-container)',
    )
  })

  it('tertiary is text-only in the primary colour, per §5', () => {
    const body = ruleFor('.mika-btn--tertiary')
    expect(body).toContain('color: var(--color-primary)')
    expect(body).toContain('background-color: transparent')
    expect(body).toContain('border: 0')
  })

  it('declares the spinner animation without relying on Tailwind', () => {
    // `animate-spin` reaches the dashboard only because a dashboard source file
    // happens to use it, and reaches the landing and mika-cloud not at all.
    expect(hasRule('.mika-spin')).toBe(true)
    expect(ruleFor('.mika-spin')).toContain('mika-spin')
    expect(THEME_CSS).toContain('@keyframes mika-spin')
  })
})

/**
 * LC.2b (mika#2562) — `<SecretField>`, `<AuthCard>` and `<Stepper>` presentation.
 *
 * Same split as LC.2 above: the component tests pin which class each state
 * emits, and these pin what those classes *mean*. Same reason for the classes
 * existing at all — Tailwind does not scan `packages/ui` — and it binds harder
 * here, because the only consumer of these three is `mika-cloud`, which imports
 * `theme.css` and none of this package's build configuration.
 *
 * Only one of the three has a rulebook section to conform to. §5 "Input Fields"
 * describes the field chrome and is asserted against literally below. The
 * *secret* grammar, the auth card and the stepper have none (mika#2562 §2.1
 * measured that), so their rules are asserted against the measured caller and
 * against the §6/§7 constraints that apply to any surface — never against an
 * invented section.
 */
describe('theme.css — LC.2b (mika#2562) rulebook §5 input-field presentation', () => {
  it('gives the field the §5 surface and border', () => {
    const body = ruleFor('.mika-field')
    // §5: "Background: `surface_container_lowest`."
    expect(body).toContain('background-color: var(--color-surface-container-lowest)')
    // §5: "Border: `outline_variant` at 10% opacity."
    expect(body).toContain('var(--color-outline-variant)')
    expect(body).toContain('10%')
  })

  it('gives the field the §6/§7 roundedness, from the token', () => {
    expect(ruleFor('.mika-field')).toContain('border-radius: var(--radius-mika-lg)')
  })

  it('renders the §5 focus state as a primary border plus a 4px primary_dim blur', () => {
    const body = ruleFor('.mika-field:focus')
    // §5: "Focus State: Border transitions to `primary` with a 4px outer
    // `primary_dim` blur at 10% opacity."
    expect(body).toContain('border-color: var(--color-primary)')
    expect(body).toContain('4px')
    expect(body).toContain('var(--color-primary-dim)')
    expect(body).toContain('10%')
  })

  it('never suppresses the focus outline outright', () => {
    // `outline: none` plus a box-shadow leaves the field with no focus indicator
    // at all in forced-colors mode, where the shadow is dropped. A transparent
    // outline is repainted by the OS palette there.
    const body = ruleFor('.mika-field:focus')
    expect(body).not.toContain('outline: none')
    expect(body).toContain('outline: 2px solid transparent')
  })

  it('makes the secret variant monospaced and clears its reveal control', () => {
    const body = ruleFor('.mika-field--secret')
    expect(body).toContain('font-family: var(--font-mono)')
    // Off-scale `pr-14` in the measured caller; snapped to the §6 scale here.
    expect(body).toContain('padding-right: var(--spacing-16)')
  })

  it('positions the reveal control without restyling it', () => {
    // The control is `<Button variant="tertiary">`, so its colour, radius and
    // focus ring come from `.mika-btn--tertiary`. This rule declaring any of
    // them would be a second, drifting definition of §5's tertiary button.
    const body = ruleFor('.mika-field-reveal')
    expect(body).toContain('position: absolute')
    expect(body).not.toContain('color:')
    expect(body).not.toContain('background')
    expect(body).not.toContain('border')
  })
})

describe('theme.css — LC.2b (mika#2562) auth card and stepper presentation', () => {
  it('layers the card one tier above the background, per §7 Do', () => {
    // §7 Do — "Layer with Intent": every nested container is at least one tier
    // from its parent. The screen is `background`; the card is
    // `surface_container`.
    expect(ruleFor('.mika-auth-card')).toContain(
      'background-color: var(--color-surface-container)',
    )
  })

  it('gives the card the §6 roundedness from the token, never a literal', () => {
    expect(ruleFor('.mika-auth-card')).toContain('border-radius: var(--radius-mika-lg)')
  })

  it('keeps the card off the viewport edge on a narrow screen', () => {
    // Absent from the measured callers. Without it the card touches the edge
    // below its own 24rem max-width, which is most phones in portrait.
    expect(ruleFor('.mika-auth-screen')).toContain('padding: var(--spacing-4)')
  })

  it('gives the header more room below it than it gives its own subtitle', () => {
    // §7 Do — "Asymmetric Whitespace".
    expect(ruleFor('.mika-auth-card__header')).toContain('margin-bottom: var(--spacing-8)')
    expect(ruleFor('.mika-auth-card__subtitle')).toContain('margin-top: var(--spacing-2)')
  })

  it.each([
    ['complete', 'background-color: var(--color-primary)'],
    ['current', 'border-color: color-mix'],
    ['upcoming', 'background-color: var(--color-surface-container-high)'],
  ])('declares the %s pill', (state, needle) => {
    expect(ruleFor(`.mika-stepper__pill--${state}`)).toContain(needle)
  })

  it('inks the completed pill on the darkest surface, never pure white', () => {
    // §7 Don't — "No Pure White". The check sits on `primary`, so its ink is a
    // surface token; the token-only scan below catches `#ffffff`, not `white`
    // written as a keyword, which is why this is asserted positively.
    expect(ruleFor('.mika-stepper__pill--complete')).toContain(
      'color: var(--color-surface-container-lowest)',
    )
  })

  it('turns the connector primary only once the step behind it is done', () => {
    expect(ruleFor('.mika-stepper__connector')).toContain(
      'background-color: var(--color-outline-variant)',
    )
    expect(ruleFor('.mika-stepper__connector--complete')).toContain('var(--color-primary)')
  })

  it('declares no hover, focus or cursor rule for a stepper that is not interactive', () => {
    // The measured caller drives the active step from server state and no pill
    // is clickable. A hover or a pointer cursor here would advertise an
    // affordance that does not exist.
    const interactive = mikaRules.filter(
      ([sel, body]) =>
        sel.includes('.mika-stepper') && (sel.includes(':hover') || body.includes('cursor:')),
    )
    expect(interactive).toEqual([])
  })

  /**
   * The classic way to get this wrong. `display: none` and `visibility: hidden`
   * both remove the node from the accessibility tree, which is the opposite of
   * what a visually-hidden label is for: `<Stepper>`'s "Completed" / "Not
   * started" text would then be announced by nothing and read by no one.
   */
  it('hides the sr-only helper from the eye without hiding it from AT', () => {
    const body = ruleFor('.mika-sr-only')
    expect(body).toContain('position: absolute')
    expect(body).toContain('clip-path: inset(50%)')
    expect(body).not.toContain('display: none')
    expect(body).not.toContain('visibility: hidden')
  })

  it.each([
    '.mika-sr-only',
    '.mika-field',
    '.mika-field--secret',
    '.mika-auth-screen',
    '.mika-auth-card',
    '.mika-stepper',
    '.mika-stepper__pill',
    '.mika-stepper__connector',
  ])('declares %s', (selector) => {
    // Anti-vacuity for this block specifically: the global population floor
    // below would still pass if every rule of this ticket were deleted.
    expect(hasRule(selector)).toBe(true)
  })
})

describe('theme.css — LC.2 (mika#1801) the .mika- rules compose tokens only', () => {
  /**
   * The load-bearing assertion of the whole LC.2 CSS block, and the reason it is
   * safe for presentation to live in this file at all.
   *
   * `bannedLegacyHexValues` above is a denylist, which is right for a token table
   * whose entire content is known values. It is green on a *new* purple written
   * in good faith — the exact regression `scripts/check-landing-tokens.sh` had to
   * switch to a shape rule to catch, one surface over. Here the rule is the same
   * shape: a `.mika-` rule may reference tokens and nothing else.
   */
  it('scans a non-empty population', () => {
    // Anti-vacuity. A renamed prefix would make every assertion below pass by
    // looking at nothing, which reads exactly like a clean file.
    //
    // The floor tracks the file: LC.2 set it at 12 against a population of 14,
    // and LC.2b (mika#2562) brought the population to ~39. Left at 12, deleting
    // every rule of this ticket would still have passed — a floor that stops
    // tracking is a floor that stops guarding.
    expect(mikaRules.length).toBeGreaterThanOrEqual(32)
  })

  it.each(mikaRules)('%s carries no colour literal', (_selector, body) => {
    expect(body).not.toMatch(/#[0-9a-f]{3,8}\b/)
    expect(body).not.toMatch(/\brgba?\s*\(/)
    expect(body).not.toMatch(/\bhsla?\s*\(/)
    // `white` / `black` are literals too, and the ones a hurried edit reaches
    // for — §7 forbids the first by name.
    expect(body).not.toMatch(/:\s*(white|black)\b/)
  })
})
