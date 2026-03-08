import { useParams, Link } from 'react-router'
import { useTraceDetail } from '../api/timeline.ts'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import { ArrowLeft } from 'lucide-react'

function eventTypeColor(type: string): string {
  switch (type) {
    case 'message':
      return 'border-blue-400/30 bg-blue-400/5'
    case 'audit':
      return 'border-amber-400/30 bg-amber-400/5'
    case 'task':
      return 'border-emerald-400/30 bg-emerald-400/5'
    default:
      return 'border-white/[0.05] bg-bg-card'
  }
}

function eventTypeLabel(type: string): string {
  switch (type) {
    case 'message':
      return 'text-blue-400'
    case 'audit':
      return 'text-amber-400'
    case 'task':
      return 'text-emerald-400'
    default:
      return 'text-muted'
  }
}

export default function TraceDetail() {
  const { traceId } = useParams<{ traceId: string }>()
  const { data, isLoading, error } = useTraceDetail(traceId ?? '')

  return (
    <div>
      <div className="flex items-center gap-3 mb-6">
        <Link
          to="/"
          className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
        >
          <ArrowLeft size={18} />
        </Link>
        <div>
          <h2 className="text-heading text-xl font-semibold">Trace Detail</h2>
          <p className="text-xs text-muted font-mono mt-0.5">{traceId}</p>
        </div>
      </div>

      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.length === 0 ? (
        <EmptyState message="No events found for this trace" />
      ) : (
        <div className="space-y-3">
          {data.map((event, i) => (
            <div
              key={i}
              className={`border rounded-xl p-4 ${eventTypeColor(event.event_type)}`}
            >
              <div className="flex items-center gap-3 mb-2">
                <span className={`text-xs font-semibold uppercase ${eventTypeLabel(event.event_type)}`}>
                  {event.event_type}
                </span>
                <span className="text-xs text-muted font-mono">
                  {event.event_subtype}
                </span>
                <span className="text-xs text-muted/60 ml-auto">
                  {formatTimestamp(event.created_at)}
                </span>
              </div>
              <div className="flex items-center gap-2 mb-2">
                <span className="text-xs text-muted/60">Agent:</span>
                <span className="text-xs text-heading">{event.agent_id}</span>
                {event.session_id && (
                  <>
                    <span className="text-xs text-muted/60 ml-2">Session:</span>
                    <Link
                      to={`/sessions/${event.session_id}`}
                      className="text-xs text-accent font-mono hover:text-accent-light transition-colors"
                    >
                      {event.session_id.slice(0, 8)}...
                    </Link>
                  </>
                )}
              </div>
              {event.summary && (
                <p className="text-sm text-muted/80 whitespace-pre-wrap break-words">
                  {event.summary}
                </p>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
