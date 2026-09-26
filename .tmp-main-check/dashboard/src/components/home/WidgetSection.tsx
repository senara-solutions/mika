import { type ReactNode } from 'react'
import { Link } from 'react-router'
import { ArrowRight } from 'lucide-react'

interface WidgetSectionProps {
  label: string
  count?: number
  viewAllTo: string
  viewAllLabel?: string
  children: ReactNode
}

/**
 * Shared wrapper for landing page widget sections.
 * Renders a section card with uppercase label, optional count badge,
 * and a "View all" link in the header.
 */
export default function WidgetSection({
  label,
  count,
  viewAllTo,
  viewAllLabel = 'View all',
  children,
}: WidgetSectionProps) {
  return (
    <section className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2.5">
          <h3 className="text-[11px] text-muted/60 font-medium uppercase tracking-wider">
            {label}
          </h3>
          {count !== undefined && count > 0 && (
            <span className="text-[10px] text-accent bg-accent/10 px-1.5 py-0.5 rounded-md font-medium">
              {count}
            </span>
          )}
        </div>
        <Link
          to={viewAllTo}
          className="flex items-center gap-1 text-[11px] text-muted/50 hover:text-accent-light transition-colors"
        >
          {viewAllLabel}
          <ArrowRight size={12} />
        </Link>
      </div>
      {children}
    </section>
  )
}
