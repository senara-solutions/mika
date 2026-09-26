import { useState } from 'react'
import { ChevronDown } from 'lucide-react'

interface CollapsibleCardProps {
  title: string
  defaultOpen?: boolean
  badge?: React.ReactNode
  children: React.ReactNode
}

export function CollapsibleCard({
  title,
  defaultOpen = true,
  badge,
  children,
}: CollapsibleCardProps) {
  const [isOpen, setIsOpen] = useState(defaultOpen)

  return (
    <div className="bg-bg-card border border-white/[0.05] rounded-2xl">
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className="flex w-full items-center justify-between p-5 text-left"
      >
        <div className="flex items-center gap-2">
          <h3 className="text-heading text-sm font-medium">{title}</h3>
          {badge}
        </div>
        <ChevronDown
          className={`h-4 w-4 text-muted/40 transition-transform duration-200 ${
            isOpen ? 'rotate-0' : '-rotate-90'
          }`}
        />
      </button>
      {isOpen && <div className="px-5 pb-5 -mt-2">{children}</div>}
    </div>
  )
}
