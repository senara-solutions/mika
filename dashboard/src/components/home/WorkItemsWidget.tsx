import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from '../../api/client.ts'
import type { TaskItem } from '../../api/tasks.ts'
import { TaskStatusBadge, LoadingState, ErrorState, EmptyState, formatApiError, formatRelativeTime } from '@senara-solutions/ui'
import WidgetSection from './WidgetSection.tsx'

interface WorkItemsWidgetProps {
  refetchInterval?: number | false
}

export default function WorkItemsWidget({ refetchInterval }: WorkItemsWidgetProps) {
  const filters = { status: 'in_progress', per_page: 5, page: 1 }
  const { data, isLoading, error, refetch } = useQuery<PaginatedResponse<TaskItem>>({
    queryKey: ['tasks', filters],
    queryFn: () => apiFetch('/tasks', filters as Record<string, string | number | undefined>),
    refetchInterval,
  })

  return (
    <WidgetSection
      label="Work Items"
      count={data?.total}
      viewAllTo="/tasks?status=in_progress"
    >
      {isLoading ? (
        <LoadingState variant="list" rows={3} />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No active work items" />
      ) : (
        <div className="space-y-1">
          {data.data.map((task) => (
            <Link
              key={task.id}
              to={`/tasks/${task.id}`}
              className="flex items-center justify-between px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors group"
            >
              <div className="min-w-0 flex-1 mr-3">
                <p className="text-sm text-heading truncate group-hover:text-accent-light transition-colors">
                  {task.label}
                </p>
                <div className="flex items-center gap-2 mt-0.5">
                  <span className="text-[10px] text-muted/40">{task.agent_id}</span>
                  <span className="text-[10px] text-muted/20">&middot;</span>
                  <span className="text-[10px] text-muted/40">{formatRelativeTime(task.updated_at)}</span>
                </div>
              </div>
              <TaskStatusBadge status={task.status} />
            </Link>
          ))}
        </div>
      )}
    </WidgetSection>
  )
}

export type { WorkItemsWidgetProps }
