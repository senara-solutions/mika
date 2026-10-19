import { AlertCircle } from 'lucide-react'
import Button from './Button.tsx'

export interface ErrorStateProps {
  message?: string
  retry?: () => void
  detailsHref?: string
  variant?: 'list' | 'detail-section'
}

export default function ErrorState({
  message = 'An unexpected error occurred.',
  retry,
  detailsHref,
  variant = 'list',
}: ErrorStateProps) {
  if (variant === 'detail-section') {
    return (
      <div role="alert" className="flex items-center gap-2 py-3 text-sm">
        <AlertCircle size={16} className="text-error/60 shrink-0" aria-hidden="true" />
        <span className="text-muted/70">{message}</span>
        {/* §5.5 l.238 describes this variant as "compact inline, 'Retry' link
            only" — which is §5's "text-only, low-priority action" verbatim, i.e.
            `tertiary`. The hand-rolled form it replaces was also never painted:
            `text-accent` and `hover:text-accent-light` are not generated for
            this package (see theme.css § Tailwind does not scan packages/ui). */}
        {retry && (
          <Button variant="tertiary" size="sm" onClick={retry}>
            Retry
          </Button>
        )}
      </div>
    )
  }

  return (
    <div role="alert" className="flex flex-col items-center justify-center py-16 text-center">
      <div className="mb-4 rounded-full bg-error/10 p-3">
        <AlertCircle size={28} className="text-error/60" aria-hidden="true" />
      </div>
      <p className="text-heading text-sm font-medium mb-1">Failed to load</p>
      <p className="text-muted/60 text-xs mb-4 max-w-md">{message}</p>
      <div className="flex items-center gap-3">
        {/* §5.5 l.272 names this button as the place of the design system's
            visual signature: "`retry` … — primary gradient button per rulebook
            §5". The hand-rolled form it replaces got all four details wrong —
            90° instead of §2's 135°, arriving at `secondary` instead of
            `primary_dim`, `text-white` against §7's "No Pure White" — and,
            because none of its utilities were generated for this package,
            probably painted no fill at all. `.mika-cta-gradient` is real CSS in
            theme.css, pinned to §2 by theme.test.ts. */}
        {retry && (
          <Button variant="primary" size="sm" onClick={retry}>
            Retry
          </Button>
        )}
        {/* `external` produces `target="_blank"` and the `noopener noreferrer`
            pair together (Button.tsx:135) — there is no state where one arrives
            without the other, which is what the hand-rolled form could not
            promise. */}
        {detailsHref && (
          <Button as="link" href={detailsHref} external variant="tertiary" size="sm">
            View error details &#8599;
          </Button>
        )}
      </div>
    </div>
  )
}
