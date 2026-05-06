import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from '../../api/client.ts'
import type { TeamRun } from '../../api/teams.ts'
import { TaskStatusBadge, LoadingState, ErrorState, EmptyState, formatApiError } from '@senara-solutions/ui'
import WidgetSection from './WidgetSection.tsx'

interface TeamRunsWidgetProps {
  refetchInterval?: number | false
}

export default function TeamRunsWidget({ refetchInterval }: TeamRunsWidgetProps) {
  const filters = { per_page: 5, page: 1 }
  const { data, isLoading, error, refetch } = useQuery<PaginatedResponse<TeamRun>>({
    queryKey: ['team-runs', filters],
    queryFn: () => apiFetch('/team-runs', filters as Record<string, string | number | undefined>),
    refetchInterval,
  })

  return (
    <WidgetSection
      label="Team Runs"
      count={data?.total}
      viewAllTo="/team-runs"
    >
      {isLoading ? (
        <LoadingState variant="list" rows={3} />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No team runs" />
      ) : (
        <div className="space-y-1">
          {data.data.map((run) => (
            <Link
              key={run.id}
              to={`/team-runs/${run.id}`}
              className="flex items-center justify-between px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors group"
            >
              <div className="min-w-0 flex-1 mr-3">
                <p className="text-sm text-heading truncate group-hover:text-accent-light transition-colors">
                  {run.team_name}
                </p>
                <div className="flex items-center gap-2 mt-0.5">
                  <span className="text-[10px] text-muted/40 truncate max-w-[250px]">{run.goal}</span>
                  <span className="text-[10px] text-muted/20">&middot;</span>
                  <span className="text-[10px] text-muted/40">{run.iteration}/{run.max_iterations}</span>
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

