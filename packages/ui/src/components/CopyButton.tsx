import { useState } from 'react'
import { Copy, Check } from 'lucide-react'

export default function CopyButton({
  text,
  className,
  title = 'Copy to clipboard',
}: {
  text: string
  className?: string
  title?: string
}) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Silently fail — clipboard may be unavailable
    }
  }

  const label = copied ? 'Copied to clipboard' : title

  return (
    <button
      onClick={handleCopy}
      className={`opacity-40 hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-1 focus-visible:ring-offset-bg focus-visible:outline-none rounded transition-opacity shrink-0 ${className ?? ''}`}
      title={title}
      aria-label={label}
      data-testid="copy-button"
    >
      <span className="relative inline-flex items-center justify-center w-3 h-3">
        <Copy
          size={12}
          data-testid="copy-icon"
          aria-hidden="true"
          className={`transition-opacity duration-150 ${copied ? 'opacity-0' : 'opacity-100'}`}
        />
        <Check
          size={12}
          data-testid="check-icon"
          aria-hidden="true"
          className={`absolute transition-opacity duration-150 text-success ${copied ? 'opacity-100' : 'opacity-0'}`}
        />
      </span>
      {/* `.mika-sr-only` and not Tailwind's `sr-only`: Tailwind does not scan
          `packages/ui`, so `sr-only` reached a consumer only where that
          consumer's own source happened to use it. Measured 2026-09-28: it is
          in the dashboard's built CSS solely because `CostTrendChart.tsx` uses
          it, and it is in neither the landing's nor mika-cloud's — where this
          live region therefore rendered as visible text. LC.2b (mika#2562)
          declares the class in `theme.css`, so routing this callsite to it is
          the fix, not a rename. */}
      <span className="mika-sr-only" role="status" aria-live="polite">
        {copied ? 'Copied' : ''}
      </span>
    </button>
  )
}
