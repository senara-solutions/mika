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
    >
      {copied ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
    </button>
  )
}
