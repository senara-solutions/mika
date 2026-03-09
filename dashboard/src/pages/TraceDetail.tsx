import { useParams, Link } from 'react-router'
import { useTraceDetail } from '../api/timeline.ts'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import { ArrowLeft } from 'lucide-react'

function eventTypeColor(type: string): string {
  switch (type) {
    case 'message':
      return 'border-blue-400/20 bg-blue-400/5'
    case 'audit':
      return 'border-amber-400/20 bg-amber-400/5'
    case 'task':
      return 'border-emerald-400/20 bg-emerald-400/5'
    default:
      return 'border-white/[0.05] bg-bg-card'
  }
}

function eventTypeBadge(type: string): { bg: string; text: string } {
  switch (type) {
    case 'message':
      return { bg: 'bg-blue-500/15', text: 'text-blue-400' }
    case 'audit':
      return { bg: 'bg-amber-500/15', text: 'text-amber-400' }
    case 'task':
      return { bg: 'bg-emerald-500/15', text: 'text-emerald-400' }
    default:
      return { bg: 'bg-white/[0.05]', text: 'text-muted' }
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
          <p className="text-xs text-accent font-mono mt-0.5">{traceId}</p>
        </div>
      </div>

      {/* Event count summary */}
      {data && data.length > 0 && (
        <div className="flex items-center gap-3 mb-5">
          <span className="text-xs text-muted/60">{data.length} events in this trace</span>
          <span className="text-white/[0.06]">|</span>
          {['message', 'audit', 'task'].map((type) => {
            const count = data.filter((e) => e.event_type === type).length
            if (count === 0) return null
            const badge = eventTypeBadge(type)
            return (
              <span
                key={type}
                className={`inline-flex items-center gap-1.5 text-[10px] font-medium px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
              >
                {count} {type}
              </span>
            )
          })}
        </div>
      )}

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
          {data.map((event, i) => {
            const badge = eventTypeBadge(event.event_type)
            return (
              <div
                key={i}
                className={`border rounded-xl p-4 ${eventTypeColor(event.event_type)}`}
              >
                <div className="flex items-center gap-3 mb-2">
                  <span
                    className={`inline-flex items-center gap-1.5 text-xs font-semibold px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
                  >
                    {event.event_type}
                  </span>
                  <span className="text-xs text-muted font-mono">
                    {event.event_subtype}
                  </span>
                  <span className="text-xs text-muted/50 ml-auto font-mono">
                    {formatTimestamp(event.created_at)}
                  </span>
                </div>
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-[10px] text-muted/40 uppercase tracking-wider">Agent</span>
                  <span className="text-xs text-heading font-medium">{event.agent_id}</span>
                  {event.session_id && (
                    <>
                      <span className="text-white/[0.06] mx-1">|</span>
                      <span className="text-[10px] text-muted/40 uppercase tracking-wider">Session</span>
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
            )
          })}
        </div>
      )}
    </div>
  )
}
