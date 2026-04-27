import { AlertCircle } from 'lucide-react'

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
        {retry && (
          <button
            type="button"
            onClick={retry}
            className="text-accent text-xs hover:text-accent-light transition-colors ml-1"
          >
            Retry
          </button>
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
        {retry && (
          <button
            type="button"
            onClick={retry}
            className="px-4 py-1.5 text-xs font-medium rounded-lg bg-gradient-to-r from-accent to-accent-light text-white hover:opacity-90 transition-opacity"
          >
            Retry
          </button>
        )}
        {detailsHref && (
          <a
            href={detailsHref}
            target="_blank"
            rel="noopener noreferrer"
            className="text-accent text-xs hover:text-accent-light transition-colors"
          >
            View error details &#8599;
          </a>
        )}
      </div>
    </div>
  )
}
