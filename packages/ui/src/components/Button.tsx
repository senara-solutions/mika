import type { ReactNode } from 'react'
import Spinner from './Spinner.tsx'

type ButtonVariant = 'primary' | 'secondary' | 'tertiary'
type ButtonSize = 'sm' | 'md' | 'lg'

interface ButtonBaseProps {
  /** Rulebook §5. Defaults to `primary`. */
  variant?: ButtonVariant
  /** Defaults to `md`. */
  size?: ButtonSize
  /** Decorative leading icon — the primitive marks it `aria-hidden`. */
  icon?: ReactNode
  children: ReactNode
  className?: string
  /** Required when the button has no visible text. */
  ariaLabel?: string
}

interface ButtonActionProps extends ButtonBaseProps {
  as?: 'button'
  onClick: () => void
  type?: 'button' | 'submit'
  disabled?: boolean
  /**
   * Implies `disabled`, and renders `<Spinner>` in place of `icon`.
   *
   * Both measured callers drive their disabled state from a mutation's pending
   * flag; keeping the two props independent would permit a loading button that
   * is still clickable — the double submit the state exists to prevent.
   */
  loading?: boolean

  /**
   * `never`, and this pair is the **only** part of the union that a hand-written
   * `never` buys anything — measured by deletion, not assumed.
   *
   * TypeScript narrows a union props type on a *present* discriminant, at a JSX
   * call site and on an object-literal assignment alike. So `as="link"` is enough
   * on its own to refuse `disabled`, and `ButtonLinkProps` needs no `never`
   * declarations at all. But `as` is **optional** on this branch — D1's
   * deliberate choice, so the majority call stays `<Button onClick={…}>` — which
   * means an action-shaped value carries *no* discriminant. TypeScript then falls
   * back to the union's excess-property rule, which admits any property declared
   * on **any** member, and `href` is declared on `ButtonLinkProps`. Without these
   * two, `<Button onClick={…} href="/x">` compiles and renders a `<button>` that
   * silently drops the `href`.
   *
   * Verified by deletion: removing them is the one edit that makes
   * `Button.type-assertions.tsx` report *"Unused '@ts-expect-error' directive"*.
   * Removing the four on the link branch changes nothing, which is why they are
   * not there.
   */
  href?: never
  external?: never
}

interface ButtonLinkProps extends ButtonBaseProps {
  as: 'link'
  href: string
  /** Adds `target="_blank"` and the `noopener noreferrer` pair. */
  external?: boolean

  /**
   * No `onClick?: never` / `disabled?: never` / `loading?: never` / `type?: never`
   * here, and that absence is a measured decision rather than an oversight.
   *
   * R-2 requires the action-only props to be *unreachable* on a link, not merely
   * ignored, and they are: `as: 'link'` is a **present** discriminant, so
   * TypeScript narrows the union to this interface — at a JSX call site and on an
   * object-literal assignment alike — and rejects `disabled` as an unknown
   * property. Adding the four `never`s was tried and deleted again: with them and
   * without them, `Button.type-assertions.tsx` reports exactly the same thing, so
   * they were four lines asserting a property something else already held.
   *
   * The mirror pair on `ButtonActionProps` is a different matter — see its own
   * note. `as` is optional there, so that branch has no discriminant and its
   * `never`s are the only thing standing between `<Button onClick href>` and a
   * silently dropped `href`.
   */
}

type ButtonProps = ButtonActionProps | ButtonLinkProps

const SPINNER_FOR_SIZE: Record<ButtonSize, 'xs' | 'sm' | 'md'> = {
  sm: 'xs',
  md: 'sm',
  lg: 'md',
}

/**
 * The library's one button, implementing rulebook §5 "Buttons".
 *
 * Presentation lives in `theme.css` (`.mika-btn*`, `.mika-cta-gradient`) rather
 * than in Tailwind utilities, because **Tailwind does not scan `packages/ui`** —
 * see that file's block comment for the measurement. Built on utilities this
 * component would ship with no padding, no radius and no focus ring.
 *
 * The union discriminates on the **rendered element**, not on `variant`. The
 * three §5 variants take exactly the same props, so a union over them would be
 * one interface under three names; the axis that carries information is
 * `<button>` versus `<a href>`, which is where the props genuinely diverge.
 * `as` is optional on the action branch so the majority call stays
 * `<Button onClick={...}>`.
 */
export default function Button(props: ButtonProps) {
  const {
    variant = 'primary',
    size = 'md',
    icon,
    children,
    className = '',
    ariaLabel,
  } = props

  const classes = [
    'mika-btn',
    `mika-btn--${variant}`,
    `mika-btn--${size}`,
    // The §2 CTA texture is declared once and composed here, so it stays usable
    // standalone by a surface the primitive does not reach.
    variant === 'primary' ? 'mika-cta-gradient' : '',
    className,
  ]
    .filter(Boolean)
    .join(' ')

  if (props.as === 'link') {
    const { href, external } = props
    return (
      <a
        href={href}
        className={classes}
        aria-label={ariaLabel}
        {...(external ? { target: '_blank', rel: 'noopener noreferrer' } : {})}
      >
        {icon ? <span aria-hidden="true">{icon}</span> : null}
        {children}
      </a>
    )
  }

  const { onClick, type = 'button', disabled = false, loading = false } = props
  const isDisabled = disabled || loading

  return (
    <button
      type={type}
      onClick={onClick}
      disabled={isDisabled}
      // Only while loading. A merely-disabled button is not busy, and saying so
      // would make the attribute useless for the state it exists to announce.
      aria-busy={loading || undefined}
      className={classes}
      aria-label={ariaLabel}
    >
      {/* The spinner replaces the icon rather than joining it, and `children`
          stays rendered, so the button keeps its width through the request —
          the layout jump a shrinking button causes is the classic regression
          here. `decorative` because the button already carries `aria-busy`:
          two nested live regions announce the same fact twice. */}
      {loading ? (
        <Spinner size={SPINNER_FOR_SIZE[size]} decorative />
      ) : icon ? (
        <span aria-hidden="true">{icon}</span>
      ) : null}
      {children}
    </button>
  )
}

export type {
  ButtonVariant,
  ButtonSize,
  ButtonProps,
  ButtonActionProps,
  ButtonLinkProps,
}
