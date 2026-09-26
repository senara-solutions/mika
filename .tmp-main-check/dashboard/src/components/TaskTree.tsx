import { useState, useMemo, type ReactNode } from 'react'
import { Link } from 'react-router'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { TaskStatusBadge, formatRelativeTime } from '@samidarko/ui'
import type { TaskItem } from '../api/tasks.ts'

interface TaskTreeProps {
  tasks: TaskItem[]
  rootTaskId: string
}

/** Build a lookup from parent_task_id → children. */
function buildChildrenMap(tasks: TaskItem[]): Map<string, TaskItem[]> {
  const map = new Map<string, TaskItem[]>()
  for (const task of tasks) {
    if (task.parent_task_id) {
      const siblings = map.get(task.parent_task_id)
      if (siblings) {
        siblings.push(task)
      } else {
        map.set(task.parent_task_id, [task])
      }
    }
  }
  return map
}

/** Mark direct children of root as expanded so their subtrees are visible on first render. */
function defaultExpanded(tasks: TaskItem[], rootTaskId: string): Set<string> {
  const expanded = new Set<string>()
  for (const task of tasks) {
    if (task.parent_task_id === rootTaskId) {
      expanded.add(task.id)
    }
  }
  return expanded
}

function TaskTreeNode({
  task,
  childrenMap,
  expanded,
  onToggle,
  indentLevel,
}: {
  task: TaskItem
  childrenMap: Map<string, TaskItem[]>
  expanded: Set<string>
  onToggle: (id: string) => void
  indentLevel: number
}) {
  const children = childrenMap.get(task.id)
  const hasChildren = !!children && children.length > 0
  const isExpanded = expanded.has(task.id)

  return (
    <>
      <div
        className="flex items-center gap-2 py-1.5 hover:bg-white/[0.02] transition-colors rounded-lg"
        style={{ paddingLeft: `${indentLevel * 24}px` }}
      >
        {/* Expand/collapse chevron */}
        {hasChildren ? (
          <button
            type="button"
            onClick={() => onToggle(task.id)}
            className="flex items-center justify-center w-5 h-5 shrink-0"
            aria-label={isExpanded ? 'Collapse' : 'Expand'}
          >
            {isExpanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted/40 transition-transform" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted/40 transition-transform" />
            )}
          </button>
        ) : (
          <span className="w-5 shrink-0" />
        )}

        {/* Status badge */}
        <TaskStatusBadge status={task.status} />

        {/* Label link */}
        <Link
          to={`/tasks/${task.id}`}
          className="text-accent text-xs hover:text-accent-light transition-colors truncate"
        >
          {task.label}
        </Link>

        {/* Right side metadata */}
        <div className="flex items-center gap-2 ml-auto shrink-0">
          {/* Trigger type badge */}
          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-muted/60">
            {task.trigger_type}
          </span>

          {/* Session link */}
          {task.created_by_session && (
            <Link
              to={`/sessions/${task.created_by_session}`}
              className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-accent hover:text-accent-light transition-colors"
            >
              SESSION
            </Link>
          )}

          {/* Trace link */}
          {task.execution_trace_id && (
            <Link
              to={`/traces/${task.execution_trace_id}`}
              className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-accent hover:text-accent-light transition-colors"
            >
              TRACE
            </Link>
          )}

          {/* Timing */}
          <span className="text-muted/40 text-xs">
            {formatRelativeTime(task.created_at)}
          </span>
        </div>
      </div>

      {/* Render children if expanded */}
      {hasChildren && isExpanded && (
        children.map((child) => (
          <TaskTreeNode
            key={child.id}
            task={child}
            childrenMap={childrenMap}
            expanded={expanded}
            onToggle={onToggle}
            indentLevel={indentLevel + 1}
          />
        ))
      )}
    </>
  )
}

export function TaskTree({ tasks, rootTaskId }: TaskTreeProps): ReactNode {
  const childrenMap = useMemo(() => buildChildrenMap(tasks), [tasks])

  const [expanded, setExpanded] = useState<Set<string>>(() =>
    defaultExpanded(tasks, rootTaskId),
  )

  const onToggle = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) {
        next.delete(id)
      } else {
        next.add(id)
      }
      return next
    })
  }

  // Top-level nodes: direct children of the root task
  const topLevel = childrenMap.get(rootTaskId) ?? []

  if (topLevel.length === 0) return null

  return (
    <div className="space-y-0">
      {topLevel.map((task) => (
        <TaskTreeNode
          key={task.id}
          task={task}
          childrenMap={childrenMap}
          expanded={expanded}
          onToggle={onToggle}
          indentLevel={0}
        />
      ))}
    </div>
  )
}
