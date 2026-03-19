import { useState } from 'react'
import { Link } from 'react-router'
import { useTasks, useTaskChildren, type TasksFilters, type TaskItem } from '../api/tasks.ts'
import { Pagination, EmptyState, TaskStatusBadge, formatRelativeTime } from '@senara-solutions/ui'
import { ChevronDown, ChevronRight } from 'lucide-react'

function TaskRow({ task, indent = 0 }: { task: TaskItem; indent?: number }) {
  return (
    <tr className="hover:bg-white/[0.02] transition-colors">
      <td className="px-4 py-3">
        <TaskStatusBadge status={task.status} />
      </td>
      <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
        {indent > 0 && (
          <span style={{ paddingLeft: `${indent * 1.5}rem` }} className="inline-block" />
        )}
        {task.label}
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
    </tr>
  )
}

function ChildTaskRows({ parentTaskId, indent }: { parentTaskId: string; indent: number }) {
  const { data: children, isLoading } = useTaskChildren(parentTaskId)

  if (isLoading) {
    return (
      <tr>
        <td colSpan={7} className="px-4 py-2 text-xs text-muted/40" style={{ paddingLeft: `${indent * 1.5 + 1}rem` }}>
          Loading...
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

  return (
    <>
      <tr
        className={`hover:bg-white/[0.02] transition-colors ${isRoot ? 'cursor-pointer' : ''}`}
        onClick={isRoot ? toggle : undefined}
      >
        <td className="px-4 py-3">
          <div className="flex items-center gap-1">
            {isRoot && (
              isExpanded
                ? <ChevronDown size={12} className="text-muted/40 shrink-0" />
                : <ChevronRight size={12} className="text-muted/40 shrink-0" />
            )}
            <TaskStatusBadge status={task.status} />
          </div>
        </td>
        <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
          {indent > 0 && (
            <span style={{ paddingLeft: `${indent * 1.5}rem` }} className="inline-block" />
          )}
          {task.label}
        </td>
        <td className="px-4 py-3">
          <Link
            to={`/agents/${task.agent_id}`}
            className="text-accent text-xs hover:text-accent-light transition-colors"
            onClick={(e) => e.stopPropagation()}
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
              onClick={(e) => e.stopPropagation()}
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
              onClick={(e) => e.stopPropagation()}
            >
              {task.created_trace_id.slice(0, 12)}...
            </Link>
          )}
        </td>
        <td className="px-4 py-3 text-xs text-muted/70 font-mono">
          {formatRelativeTime(task.updated_at)}
        </td>
      </tr>
      {isExpanded && <ChildTaskRows parentTaskId={task.id} indent={indent + 1} />}
    </>
  )
}

function TaskTable({
  headers,
  children,
}: {
  headers: string[]
  children: React.ReactNode
}) {
  return (
    <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
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

function WorkItemsSection() {
  const [page, setPage] = useState(1)
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const filters: TasksFilters = { trigger_type: 'manual', page, per_page: 20 }
  const { data, isLoading, error } = useTasks(filters)

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
        <div className="text-muted/60 py-4 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-4 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No active work items" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Source', 'Team Run', 'Trace', 'Updated']}
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

function TeamRunTasksSection() {
  const [page, setPage] = useState(1)
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set())
  const filters: TasksFilters = {
    team_run_id: 'notnull',
    page,
    per_page: 20,
  }
  const { data, isLoading, error } = useTasks(filters)

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
        <div className="text-muted/60 py-4 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-4 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No team run tasks" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Action', 'Team Run', 'Trace', 'Updated']}
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

function StandaloneCallbacksSection() {
  const [page, setPage] = useState(1)
  const filters: TasksFilters = {
    action_type: 'resume_agent,run_skill',
    team_run_id: 'null',
    trigger_type: 'callback',
    page,
    per_page: 20,
  }
  const { data, isLoading, error } = useTasks(filters)

  return (
    <Section title="Standalone Callbacks" count={data?.total} defaultOpen={true}>
      {isLoading ? (
        <div className="text-muted/60 py-4 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-4 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
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

function ScheduledSection() {
  const [page, setPage] = useState(1)
  const filters: TasksFilters = {
    trigger_type: 'cron,one_shot',
    page,
    per_page: 20,
  }
  const { data, isLoading, error } = useTasks(filters)

  return (
    <Section title="Scheduled Tasks" count={data?.total} defaultOpen={false}>
      {isLoading ? (
        <div className="text-muted/60 py-4 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-4 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No scheduled tasks" />
      ) : (
        <>
          <TaskTable
            headers={['Status', 'Title', 'Agent', 'Schedule', 'Next Run', 'Last Run', 'Updated']}
          >
            {data.data.map((t) => (
              <tr key={t.id} className="hover:bg-white/[0.02] transition-colors">
                <td className="px-4 py-3">
                  <TaskStatusBadge status={t.status} />
                </td>
                <td className="px-4 py-3 text-xs text-heading max-w-[300px] truncate">
                  {t.label}
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
              </tr>
            ))}
          </TaskTable>
          <Pagination page={page} perPage={20} total={data.total} onPageChange={setPage} />
        </>
      )}
    </Section>
  )
}

export default function Tasks() {
  return (
    <div>
      <div className="mb-5">
        <h2 className="text-heading text-xl font-semibold">Tasks</h2>
        <p className="text-sm text-muted/60 mt-1">
          Monitor work items, team runs, callbacks, and scheduled tasks
        </p>
      </div>

      <WorkItemsSection />
      <TeamRunTasksSection />
      <StandaloneCallbacksSection />
      <ScheduledSection />
    </div>
  )
}
