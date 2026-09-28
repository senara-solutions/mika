import type { ReactNode } from 'react'

interface AuthCardProps {
  /** The card's heading, rendered as the page's `<h1>`. */
  title: string
  subtitle?: string
  /**
   * Consumer-injected brand mark.
   *
   * A slot and not an import, for the reason `<AgentFilter>` takes `agents`
   * rather than calling `useAgents()`: the library cannot depend on a
   * consumer's own components. `mika-cloud` passes its `<Logo />`.
   */
  logo?: ReactNode
  /**
   * Free content, and measurably so: `Login.tsx` puts a full-width CTA and a
   * "Don't have an account? Sign up" link here, `Signup.tsx` the mirror pair.
   * Nothing about that shape is common enough to become an API.
   */
  children: ReactNode
  className?: string
}

/**
 * The centred card of the authentication screens, implementing the shape both
 * measured callers carry identically (`mika-cloud` `web/src/pages/Login.tsx`
 * and `web/src/pages/Signup.tsx`, read at `origin/main` 5cb2544).
 *
 * It owns the full-height centring as well as the card, because both callers
 * write the same `flex min-h-screen items-center justify-center` around it —
 * the "centred" half of the `AuthCard` / `CenteredCard` pair the ticket left
 * open. `AuthCard` is the name kept: the header grammar it imposes (logo,
 * `<h1>`, muted subtitle) is auth-shaped, and naming it for its shape would
 * invite it onto surfaces that want the box without the grammar.
 *
 * **What it deliberately does not own:** the "Powered by Senara Solutions"
 * footer. It exists on `Login` only, sits outside the card, and is fixed to the
 * bottom of the viewport — three reasons it belongs to the page. A `footer`
 * prop would have been an API for one of the two callers.
 *
 * Presentation lives in `theme.css` (`.mika-auth-*`), like `<Button>`'s and for
 * the same measured reason: Tailwind does not scan `packages/ui`.
 */
export default function AuthCard({
  title,
  subtitle,
  logo,
  children,
  className = '',
}: AuthCardProps) {
  return (
    <div className="mika-auth-screen">
      <div className={`mika-auth-card ${className}`.trim()}>
        <div className="mika-auth-card__header">
          {logo ? <div className="mika-auth-card__logo">{logo}</div> : null}
          {/*
            `<h1>` is not configurable. On both measured callers this card *is*
            the page, so its title is the page's only heading; letting a caller
            demote it to `<h2>` would produce a document with no `<h1>`, which
            is the accessibility defect the fixed level exists to prevent.
          */}
          <h1 className="mika-auth-card__title">{title}</h1>
          {subtitle ? <p className="mika-auth-card__subtitle">{subtitle}</p> : null}
        </div>
        {children}
      </div>
    </div>
  )
}

export type { AuthCardProps }
