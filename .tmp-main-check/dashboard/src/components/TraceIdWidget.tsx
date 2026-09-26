import { Link } from 'react-router'
import { CopyButton } from '@samidarko/ui'
import { Activity } from 'lucide-react'

interface TraceIdWidgetProps {
  traceId: string
  /** Optional LLM call count shown as a mini badge */
  llmCallCount?: number
  className?: string
}

/**
 * Trace ID chip — displays a truncated trace ID with copy + link affordances.
 * Uses CopyButton from @samidarko/ui per mika#665 AC3 constraint.
 */
export default function TraceIdWidget({ traceId, llmCallCount, className = '' }: TraceIdWidgetProps) {
  return (
    <div className={`inline-flex items-center gap-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md px-2 py-1 ${className}`}>
      <Activity size={12} className="text-accent/70 shrink-0" />
      <Link
        to={`/traces/${traceId}`}
        className="text-xs font-mono text-accent hover:text-accent-light transition-colors truncate max-w-[120px]"
        title={traceId}
      >
        {traceId.slice(0, 12)}...
      </Link>
      {llmCallCount !== undefined && llmCallCount > 0 && (
        <span className="text-[10px] text-muted/60 bg-white/[0.06] rounded px-1">
          {llmCallCount}
        </span>
      )}
      <CopyButton text={traceId} title="Copy trace ID" className="shrink-0" />
    </div>
  )
}
