import Button from './Button.tsx'
import type { ButtonProps } from './Button.tsx'

/**
 * Type-level assertions for `<Button>`'s discriminated union (mika#1801 R-2).
 *
 * WHY THIS FILE EXISTS. The plan's Definition of Done requires that
 * `<Button as="link" disabled>` be **non-compilable** — the props that have no
 * meaning on an `<a href>` must be *unreachable*, not merely ignored. No runtime
 * test can observe that: `render()` sees whatever the transpiler emitted, and
 * esbuild (which vitest uses) strips types without checking them. So the
 * assertion has to be a compile-time one, and it has to be **run**.
 *
 * `npm run typecheck -w packages/ui` is what runs it, and CI runs that.
 *
 * WHY `@ts-expect-error` RATHER THAN A CONDITIONAL-TYPE HELPER. The directive is
 * self-cleaning in the direction that matters: if someone deletes the `never`
 * declarations from `ButtonLinkProps`, these lines stop erroring and `tsc`
 * reports *"Unused '@ts-expect-error' directive"* — the build goes red on the day
 * the protection is removed, not months later. A `type Expect<...>` helper would
 * have to be kept in sync by hand and can rot silently.
 *
 * WHY THE POSITIVE CONTROLS BELOW ARE NOT DECORATION. Without them, "the union
 * refuses the contradictions" is indistinguishable from "the union refuses
 * everything" — a `ButtonProps` accidentally narrowed to `never` would satisfy
 * every negative case here and pass. That is the shape the house keeps having to
 * re-learn (mika#2205): a guard that refuses everything reads exactly like a
 * guard that is working.
 *
 * Nothing here is exported from `index.ts`, and nothing is rendered.
 */

// ── Positive controls. These must COMPILE. See the note above.
export const accepted = [
  // The majority call: no `as` needed on the action branch.
  <Button onClick={() => {}}>Search</Button>,
  <Button variant="secondary" size="lg" onClick={() => {}}>
    Read the Docs
  </Button>,
  <Button variant="tertiary" size="sm" onClick={() => {}} type="submit">
    Cancel
  </Button>,
  // The action branch's own states.
  <Button onClick={() => {}} disabled>
    Promote
  </Button>,
  <Button onClick={() => {}} loading>
    Promote
  </Button>,
  // The link branch.
  <Button as="link" href="/getting-started">
    Get Started
  </Button>,
  <Button as="link" href="https://example.com" external>
    Docs
  </Button>,
  // `variant`, `size`, `icon`, `className` and `ariaLabel` are shared, so they
  // must reach both branches.
  <Button as="link" href="/x" variant="secondary" size="lg" className="w-full">
    Get Started
  </Button>,
  <Button onClick={() => {}} ariaLabel="Search traces" icon={<svg />}>
    {''}
  </Button>,
]

// ── Negative controls. Each of these must NOT compile.
export const refused = [
  // The founding case from the Definition of Done. A plain union of the two
  // interfaces would ACCEPT this: TypeScript's excess-property check against a
  // union admits any property declared on any member, so `disabled` would be
  // waved through and render an inert <a> never announced as disabled to
  // assistive technology. The `disabled?: never` on `ButtonLinkProps` is what
  // makes it unrepresentable.
  // @ts-expect-error `disabled` is unreachable on the link branch.
  <Button as="link" href="/x" disabled>
    Get Started
  </Button>,

  // @ts-expect-error `loading` is unreachable on the link branch — a link cannot be in flight.
  <Button as="link" href="/x" loading>
    Get Started
  </Button>,

  // @ts-expect-error `type` is a <button> attribute; it means nothing on an <a>.
  <Button as="link" href="/x" type="submit">
    Get Started
  </Button>,

  // @ts-expect-error `onClick` on the link branch: navigation is what `href` is for.
  <Button as="link" href="/x" onClick={() => {}}>
    Get Started
  </Button>,

  // The mirror direction — the action branch must not take link props either,
  // or `<Button onClick={...} href="...">` would render a <button> that silently
  // drops the href.
  // @ts-expect-error `href` is unreachable on the action branch.
  <Button onClick={() => {}} href="/x">
    Go
  </Button>,

  // @ts-expect-error `external` is unreachable on the action branch.
  <Button onClick={() => {}} external>
    Go
  </Button>,

  // @ts-expect-error the link branch requires `href`.
  <Button as="link">Get Started</Button>,

  // @ts-expect-error the action branch requires `onClick`.
  <Button>Go</Button>,

  // @ts-expect-error `variant` is closed over the three §5 variants; a fourth is
  // a rulebook §8 decision reserved to Vincent, not a prop value.
  <Button variant="confirm" onClick={() => {}}>
    Promote
  </Button>,

  // @ts-expect-error `size` is closed over the three declared sizes.
  <Button size="xl" onClick={() => {}}>
    Go
  </Button>,
]

/**
 * ── The second path: a props object built up and then spread, with no JSX
 * attribute list for TypeScript to check.
 *
 * Both paths are covered because they are checked by *different* rules, and the
 * difference is what the implementation had to measure rather than assume:
 *
 *   - With a **present** discriminant (`as: 'link'`), TypeScript narrows the union
 *     to one member and applies the excess-property check against that member
 *     alone. This happens for JSX attributes and for object-literal assignment
 *     alike, which is why `ButtonLinkProps` carries no `never` declarations: they
 *     were written, measured to change nothing, and deleted.
 *   - With **no** discriminant — an action-shaped value, since D1 makes `as`
 *     optional there so the majority call stays `<Button onClick={…}>` — there is
 *     nothing to narrow on, and the union's excess-property rule admits any
 *     property declared on any member. That is the hole `href?: never` /
 *     `external?: never` close, and deleting them is the one edit that makes this
 *     file report *"Unused '@ts-expect-error' directive"*.
 */
const linkProps: ButtonProps = {
  as: 'link',
  href: '/getting-started',
  external: true,
  children: 'Get Started',
}

const actionProps: ButtonProps = {
  onClick: () => {},
  disabled: true,
  loading: false,
  type: 'submit',
  children: 'Promote',
}

export const spreadable = [
  <Button {...linkProps} />,
  <Button {...actionProps} />,
]

export const refusedAsObject: ButtonProps[] = [
  // @ts-expect-error a link-shaped object may not carry the action-only props.
  { as: 'link', href: '/x', disabled: true, children: 'Get Started' },

  // @ts-expect-error nor may it carry `loading`.
  { as: 'link', href: '/x', loading: true, children: 'Get Started' },

  // @ts-expect-error an action-shaped object may not carry `href`.
  { onClick: () => {}, href: '/x', children: 'Go' },

  // @ts-expect-error nor `external`.
  { onClick: () => {}, external: true, children: 'Go' },
]
