import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from '../../api/client.ts'
import type { DevRun } from '../../api/devRuns.ts'
import { TaskStatusBadge, LoadingState, ErrorState, EmptyState, formatApiError, formatRelativeTime } from '@samidarko/ui'
import WidgetSection from './WidgetSection.tsx'

interface DevRunsWidgetProps {
  refetchInterval?: number | false
}

export default function DevRunsWidget({ refetchInterval }: DevRunsWidgetProps) {
  const filters = { per_page: 5, page: 1 }
  const { data, isLoading, error, refetch } = useQuery<PaginatedResponse<DevRun>>({
    queryKey: ['dev-runs', filters],
    queryFn: () => apiFetch('/dev-runs', filters as Record<string, string | number | undefined>),
    refetchInterval,
  })

  return (
    <WidgetSection
      label="Dev Runs"
      count={data?.total}
      viewAllTo="/dev-runs"
    >
      {isLoading ? (
        <LoadingState variant="list" rows={3} />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No dev runs" />
      ) : (
        <div className="space-y-1">
          {data.data.map((run) => (
            <Link
              key={run.id}
              to={`/dev-runs/${run.id}`}
              className="flex items-center justify-between px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors group"
            >
              <div className="min-w-0 flex-1 mr-3">
                <p className="text-sm text-heading truncate group-hover:text-accent-light transition-colors">
                  {run.label}
                </p>
                <div className="flex items-center gap-2 mt-0.5">
                  {run.branch && (
                    <>
                      <span className="text-[10px] text-accent/60 font-mono truncate max-w-[200px]">{run.branch}</span>
                      <span className="text-[10px] text-muted/20">&middot;</span>
                    </>
                  )}
                  {run.pr_url ? (
                    <a
                      href={run.pr_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-[10px] text-accent hover:text-accent-light"
                      onClick={(e) => e.stopPropagation()}
                    >
                      PR #{run.pr_number}
                    </a>
                  ) : (
                    <span className="text-[10px] text-muted/40">{formatRelativeTime(run.created_at)}</span>
                  )}
                </div>
              </div>
              <TaskStatusBadge status={run.status} />
            </Link>
          ))}
        </div>
      )}
    </WidgetSection>
  )
}

