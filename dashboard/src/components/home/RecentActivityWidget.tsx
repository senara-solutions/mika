import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from '../../api/client.ts'
import type { TimelineRow } from '../../api/timeline.ts'
import { LoadingState, ErrorState, EmptyState, formatApiError, formatRelativeTime, eventTypeBadge } from '@senara-solutions/ui'
import WidgetSection from './WidgetSection.tsx'

interface RecentActivityWidgetProps {
  refetchInterval?: number | false
}

/** Map timeline row to the most likely detail page route. */
function eventDetailPath(row: TimelineRow): string | null {
  if (row.session_id) return `/sessions/${row.session_id}`
  if (row.trace_id) return `/traces/${row.trace_id}`
  return null
}

export default function RecentActivityWidget({ refetchInterval }: RecentActivityWidgetProps) {
  const filters = { per_page: 8, page: 1 }
  const { data, isLoading, error, refetch } = useQuery<PaginatedResponse<TimelineRow>>({
    queryKey: ['timeline', filters],
    queryFn: () => apiFetch('/timeline', filters as Record<string, string | number | undefined>),
    refetchInterval,
  })

  return (
    <WidgetSection
      label="Recent Activity"
      count={data?.total}
      viewAllTo="/timeline"
    >
      {isLoading ? (
        <LoadingState variant="list" rows={4} />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No recent activity" />
      ) : (
        <div className="space-y-1">
          {data.data.map((row, i) => {
            const badge = eventTypeBadge(row.event_type)
            const detailPath = eventDetailPath(row)
            const content = (
              <div className="flex items-center justify-between px-3 py-2 rounded-xl hover:bg-white/[0.03] transition-colors group">
                <div className="flex items-center gap-3 min-w-0 flex-1">
                  <span
                    className={`inline-flex items-center gap-1 text-[10px] font-medium px-1.5 py-0.5 rounded-full shrink-0 ${badge.bg} ${badge.text}`}
                  >
                    <span className={`w-1 h-1 rounded-full ${badge.dot}`} />
                    {badge.label}
                  </span>
                  <span className="text-xs text-muted/70 truncate">
                    {row.agent_id && (
                      <span className="text-heading font-medium mr-1.5">{row.agent_id}</span>
                    )}
                    {row.summary ?? row.event_subtype}
                  </span>
                </div>
                <span className="text-[10px] text-muted/40 shrink-0 ml-2">
                  {formatRelativeTime(row.created_at)}
                </span>
              </div>
            )

            return detailPath ? (
              <Link key={i} to={detailPath}>
                {content}
              </Link>
            ) : (
              <div key={i}>
                {content}
              </div>
            )
          })}
        </div>
      )}
    </WidgetSection>
  )
}

