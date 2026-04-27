import { useState } from 'react'
import { Link } from 'react-router'
import { useTasks, useTaskChildren, type TasksFilters, type TaskItem } from '../api/tasks.ts'
import { Pagination, EmptyState, LoadingState, ErrorState, formatApiError, TaskStatusBadge, ListRow, TimeRangeFilter, formatRelativeTime } from '@senara-solutions/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { ChevronDown, ChevronRight } from 'lucide-react'

interface TimeRangeProps {
  from?: string
  to?: string
}

function TaskRow({ task, indent = 0 }: { task: TaskItem; indent?: number }) {
  return (
    <ListRow variant="static">
      <td className="px-4 py-3">
        <TaskStatusBadge status={task.status} />
      </td>
      <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
        {indent > 0 && (
          <span style={{ paddingLeft: `${indent * 1.5}rem` }} className="inline-block" />
        )}
        <Link
          to={`/tasks/${task.id}`}
          className="text-accent hover:text-accent-light transition-colors"
        >
          {task.label}
        </Link>
      </td>
      <td className="px-4 py-3">
        <Link
          to={`/agents/${task.agent_id}`}
          className="text-accent text-xs hover:text-accent-light transition-colors"
        >
          {task.agent_id}
        </Link>
      </td>
      <td className="px-4 py-3">
        {task.source && (
          <span className="text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
            {task.source}
          </span>
        )}
      </td>
      <td className="px-4 py-3">
        {task.team_run_id ? (
          <Link
            to={`/team-runs/${task.team_run_id}`}
            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
          >
            Team Run &rarr;
          </Link>
        ) : (
          <span className="text-xs text-muted/40">&mdash;</span>
        )}
      </td>
      <td className="px-4 py-3 text-xs text-muted/70">
        {task.created_trace_id && (
          <Link
            to={`/traces/${task.created_trace_id}`}
            className="text-accent font-mono hover:text-accent-light transition-colors"
          >
            {task.created_trace_id.slice(0, 12)}...
          </Link>
        )}
      </td>
      <td className="px-4 py-3 text-xs text-muted/70 font-mono">
        {formatRelativeTime(task.updated_at)}
      </td>
    </ListRow>
  )
}

function ChildTaskRows({ parentTaskId, indent }: { parentTaskId: string; indent: number }) {
  const { data: children, isLoading } = useTaskChildren(parentTaskId)

  if (isLoading) {
    return (
      <tr>
        <td colSpan={8} className="px-4 py-2" style={{ paddingLeft: `${indent * 1.5 + 1}rem` }}>
          <div className="h-4 w-48 animate-pulse rounded bg-white/[0.06]" />
        </td>
      </tr>
    )
  }

  if (!children || children.length === 0) return null

  return (
    <>
      {children.map((child) => (
        <ExpandableTaskRow key={child.id} task={child} indent={indent} />
      ))}
    </>
  )
}

function ExpandableTaskRow({
  task,
  indent = 0,
  expandedIds,
  onToggle,
}: {
  task: TaskItem
  indent?: number
  expandedIds?: Set<string>
  onToggle?: (id: string) => void
}) {
  // When used recursively (from ChildTaskRows), manage own expanded state
  const [localExpanded, setLocalExpanded] = useState(false)
  const isExpanded = expandedIds ? expandedIds.has(task.id) : localExpanded
  const toggle = () => {
    if (onToggle) onToggle(task.id)
    else setLocalExpanded((v) => !v)
  }

  const isRoot = task.depth === 0

  const cells = (
    <>
      <td className="px-4 py-3">
        <TaskStatusBadge status={task.status} />
      </td>
      <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
        {indent > 0 && (
          <span style={{ paddingLeft: `${indent * 1.5}rem` }} className="inline-block" />
        )}
        <Link
          to={`/tasks/${task.id}`}
          className="text-accent hover:text-accent-light transition-colors"
        >
          {task.label}
        </Link>
      </td>
      <td className="px-4 py-3">
        <Link
          to={`/agents/${task.agent_id}`}
          className="text-accent text-xs hover:text-accent-light transition-colors"
        >
          {task.agent_id}
        </Link>
      </td>
      <td className="px-4 py-3">
        {task.source && (
          <span className="text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
            {task.source}
          </span>
        )}
      </td>
      <td className="px-4 py-3">
        {task.team_run_id ? (
          <Link
            to={`/team-runs/${task.team_run_id}`}
            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
          >
            Team Run &rarr;
          </Link>
        ) : (
          <span className="text-xs text-muted/40">&mdash;</span>
        )}
      </td>
      <td className="px-4 py-3 text-xs text-muted/70">
        {task.created_trace_id && (
          <Link
            to={`/traces/${task.created_trace_id}`}
            className="text-accent font-mono hover:text-accent-light transition-colors"
          >
            {task.created_trace_id.slice(0, 12)}...
          </Link>
        )}
      </td>
      <td className="px-4 py-3 text-xs text-muted/70 font-mono">
        {formatRelativeTime(task.updated_at)}
      </td>
    </>
  )

  return (
    <>
      {isRoot ? (
        <ListRow
          variant="expandable"
          isExpanded={isExpanded}
          onToggle={toggle}
          ariaLabel={`Toggle subtasks of ${task.label}`}
        >
          {cells}
        </ListRow>
      ) : (
        <ListRow variant="static">
          <td className="w-8" />{/* spacer to align with expandable rows' chevron column */}
          {cells}
        </ListRow>
      )}
      {isExpanded && <ChildTaskRows parentTaskId={task.id} indent={indent + 1} />}
    </>
  )
}

function TaskTable({
  headers,
  hasExpandableRows = false,
  children,
}: {
  headers: string[]
  hasExpandableRows?: boolean
  children: React.ReactNode
}) {
  return (
    <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
            {hasExpandableRows && <th className="w-8 px-2 py-3" />}
            {headers.map((h) => (
              <th key={h} className="text-left px-4 py-3 font-medium">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-white/[0.03]">{children}</tbody>
      </table>
    </div>
  )
}

function Section({
  title,
  count,
  defaultOpen = true,
  children,
}: {
  title: string
  count: number | undefined
  defaultOpen?: boolean
  children: React.ReactNode
}) {
  const [open, setOpen] = useState(defaultOpen)

  return (
    <div className="mb-6">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 mb-3 group w-full text-left"
      >
        {open ? (
          <ChevronDown size={16} className="text-muted/60" />
        ) : (
          <ChevronRight size={16} className="text-muted/60" />
        )}
        <h3 className="text-heading text-base font-semibold">{title}</h3>
        {count !== undefined && (
          <span className="text-xs bg-accent/10 text-accent px-2 py-0.5 rounded-full font-medium">
            {count}
          </span>
        )}
      </button>
      {open && children}
    </div>
  )
}

function WorkItemsSection({ timeRange }: { timeRange: TimeRangeProps }) {
  const [page, setPage] = useState(1)
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const filters: TasksFilters = { trigger_type: 'manual', from: timeRange.from, to: timeRange.to, page, per_page: 20 }
  const { data, isLoading, error, refetch } = useTasks(filters)

  const toggleExpanded = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  return (
    <Section title="Work Items" count={data?.total} defaultOpen={true}>
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No active work items" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Source', 'Team Run', 'Trace', 'Updated']}
            hasExpandableRows
          >
            {data.data.map((t) => (
              <ExpandableTaskRow
                key={t.id}
                task={t}
                expandedIds={expandedIds}
                onToggle={toggleExpanded}
              />
            ))}
          </TaskTable>
          <Pagination page={page} perPage={20} total={data.total} onPageChange={setPage} />
        </>
      )}
    </Section>
  )
}

function TeamRunTasksSection({ timeRange }: { timeRange: TimeRangeProps }) {
  const [page, setPage] = useState(1)
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const filters: TasksFilters = {
    team_run_id: 'notnull',
    from: timeRange.from,
    to: timeRange.to,
    page,
    per_page: 20,
  }
  const { data, isLoading, error, refetch } = useTasks(filters)

  const toggleExpanded = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  return (
    <Section title="Team Run Tasks" count={data?.total} defaultOpen={true}>
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No team run tasks" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Action', 'Team Run', 'Trace', 'Updated']}
            hasExpandableRows
          >
            {data.data.map((t) => (
              <ExpandableTaskRow
                key={t.id}
                task={t}
                expandedIds={expandedIds}
                onToggle={toggleExpanded}
              />
            ))}
          </TaskTable>
          <Pagination page={page} perPage={20} total={data.total} onPageChange={setPage} />
        </>
      )}
    </Section>
  )
}

function StandaloneCallbacksSection({ timeRange }: { timeRange: TimeRangeProps }) {
  const [page, setPage] = useState(1)
  const filters: TasksFilters = {
    action_type: 'resume_agent,run_skill',
    team_run_id: 'null',
    trigger_type: 'callback',
    from: timeRange.from,
    to: timeRange.to,
    page,
    per_page: 20,
  }
  const { data, isLoading, error, refetch } = useTasks(filters)

  return (
    <Section title="Standalone Callbacks" count={data?.total} defaultOpen={true}>
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No standalone callbacks" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Action', 'Team Run', 'Trace', 'Updated']}
          >
            {data.data.map((t) => (
              <TaskRow key={t.id} task={t} />
            ))}
          </TaskTable>
          <Pagination page={page} perPage={20} total={data.total} onPageChange={setPage} />
        </>
      )}
    </Section>
  )
}

function ScheduledSection({ timeRange }: { timeRange: TimeRangeProps }) {
  const [page, setPage] = useState(1)
  const filters: TasksFilters = {
    trigger_type: 'cron,one_shot',
    from: timeRange.from,
    to: timeRange.to,
    page,
    per_page: 20,
  }
  const { data, isLoading, error, refetch } = useTasks(filters)

  return (
    <Section title="Scheduled Tasks" count={data?.total} defaultOpen={false}>
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No scheduled tasks" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Schedule', 'Next Run', 'Last Run', 'Updated']}
          >
            {data.data.map((t) => (
              <ListRow key={t.id} variant="static">
                <td className="px-4 py-3">
                  <TaskStatusBadge status={t.status} />
                </td>
                <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
                  <Link
                    to={`/tasks/${t.id}`}
                    className="text-accent hover:text-accent-light transition-colors"
                  >
                    {t.label}
                  </Link>
                </td>
                <td className="px-4 py-3">
                  <Link
                    to={`/agents/${t.agent_id}`}
                    className="text-accent text-xs hover:text-accent-light transition-colors"
                  >
                    {t.agent_id}
                  </Link>
                </td>
                <td className="px-4 py-3 text-xs text-muted font-mono">
                  {t.cron_expr ?? 'one-shot'}
                </td>
                <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                  {t.next_fire_at ? formatRelativeTime(t.next_fire_at) : '\u2014'}
                </td>
                <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                  {t.fired_at ? formatRelativeTime(t.fired_at) : '\u2014'}
                </td>
                <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                  {formatRelativeTime(t.updated_at)}
                </td>
              </ListRow>
            ))}
          </TaskTable>
          <Pagination page={page} perPage={20} total={data.total} onPageChange={setPage} />
        </>
      )}
    </Section>
  )
}

export default function Tasks() {
  const { searchParams, setSearchParams, updateFilter } = useSearchParamsFilter()

  const timeRange: TimeRangeProps = {
    from: searchParams.get('from') ?? undefined,
    to: searchParams.get('to') ?? undefined,
  }

  return (
    <div>
      <div className="mb-5">
        <h2 className="text-heading text-xl font-semibold">Tasks</h2>
        <p className="text-sm text-muted/60 mt-1">
          Monitor work items, team runs, callbacks, and scheduled tasks
        </p>
      </div>

      {/* Time range filter bar */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <TimeRangeFilter
            value={{ from: timeRange.from, to: timeRange.to }}
            onChange={(range) => {
              updateFilter('from', range.from ?? '')
              updateFilter('to', range.to ?? '')
            }}
          />
          {(timeRange.from || timeRange.to) && (
            <button
              onClick={() => setSearchParams(new URLSearchParams())}
              className="text-xs text-muted/60 hover:text-muted transition-colors"
            >
              Clear
            </button>
          )}
        </div>
      </div>

      <WorkItemsSection timeRange={timeRange} />
      <TeamRunTasksSection timeRange={timeRange} />
      <StandaloneCallbacksSection timeRange={timeRange} />
      <ScheduledSection timeRange={timeRange} />
    </div>
  )
}
