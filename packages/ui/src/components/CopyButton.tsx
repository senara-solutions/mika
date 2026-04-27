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

  return (
    <button
      onClick={handleCopy}
      className={`opacity-40 hover:opacity-100 transition-opacity shrink-0 ${className ?? ''}`}
      title={title}
      data-testid="copy-button"
    >
      <span className="relative inline-flex items-center justify-center w-3 h-3">
        <Copy
          size={12}
          data-testid="copy-icon"
          className={`transition-opacity duration-150 ${copied ? 'opacity-0' : 'opacity-100'}`}
        />
        <Check
          size={12}
          data-testid="check-icon"
          className={`absolute transition-opacity duration-150 text-emerald-400 ${copied ? 'opacity-100' : 'opacity-0'}`}
        />
      </span>
    </button>
  )
}
