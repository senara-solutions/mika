import { Link, useParams } from 'react-router'
import { useTask, useTaskChildren, useTaskSessions } from '../api/tasks.ts'
import { TaskStatusBadge, CopyButton, EmptyState, LoadingState, ErrorState, formatApiError, formatRelativeTime } from '@samidarko/ui'
import { MetadataRow } from '../components/MetadataRow.tsx'

function formatDuration(start: string, end: string): string {
  const ms = new Date(end).getTime() - new Date(start).getTime()
  if (ms < 1000) return `${ms}ms`
  const secs = Math.floor(ms / 1000)
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  const remSecs = secs % 60
  if (mins < 60) return `${mins}m ${remSecs}s`
  const hours = Math.floor(mins / 60)
  const remMins = mins % 60
  return `${hours}h ${remMins}m`
}

export default function TaskDetail() {
  const { taskId } = useParams<{ taskId: string }>()
  const { data: task, isLoading, error, refetch } = useTask(taskId)
  const { data: children } = useTaskChildren(taskId)
  const { data: taskSessions } = useTaskSessions(taskId)

  if (isLoading) {
    return <LoadingState variant="detail" />
  }
  if (error) {
    return <ErrorState message={formatApiError(error)} retry={() => refetch()} />
  }
  if (!task) {
    return <EmptyState message="Task not found" />
  }

  return (
    <div>
      <div className="mb-5">
        <Link to="/tasks" className="text-muted/60 text-xs hover:text-muted transition-colors">
          &larr; Back to Tasks
        </Link>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h2 className="text-heading text-xl font-semibold">{task.label}</h2>
          <div className="flex items-center gap-3 mt-2">
            <TaskStatusBadge status={task.status} />
            <CopyButton text={task.id} title="Copy ID" />
          </div>
        </div>
      </div>

      {/* Task Metadata */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Task Details</h3>
        <MetadataRow label="ID">
          <span className="font-mono text-xs">{task.id}</span>
        </MetadataRow>
        <MetadataRow label="Trigger">
          <span className="text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
            {task.trigger_type}
          </span>
        </MetadataRow>
        <MetadataRow label="Action">
          <span className="text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
            {task.action_type}
          </span>
        </MetadataRow>
        {task.source && (
          <MetadataRow label="Source">
            <span className="text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
              {task.source}
            </span>
          </MetadataRow>
        )}
        <MetadataRow label="Depth">{task.depth}</MetadataRow>
        <MetadataRow label="Agent">
          <Link
            to={`/agents/${task.agent_id}`}
            className="text-accent text-xs hover:text-accent-light transition-colors"
          >
            {task.agent_id}
          </Link>
        </MetadataRow>
        {task.created_by_session && (
          <MetadataRow label="Session">
            <Link
              to={`/sessions/${task.created_by_session}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {task.created_by_session}
            </Link>
          </MetadataRow>
        )}
        {task.created_trace_id && (
          <MetadataRow label="Created Trace">
            <Link
              to={`/traces/${task.created_trace_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {task.created_trace_id.slice(0, 12)}...
            </Link>
          </MetadataRow>
        )}
        {task.execution_trace_id && (
          <MetadataRow label="Exec Trace">
            <Link
              to={`/traces/${task.execution_trace_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {task.execution_trace_id.slice(0, 12)}...
            </Link>
          </MetadataRow>
        )}
        {task.parent_task_id && (
          <MetadataRow label="Parent Task">
            <Link
              to={`/tasks/${task.parent_task_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {task.parent_task_id.slice(0, 12)}...
            </Link>
          </MetadataRow>
        )}
        {task.team_run_id && (
          <MetadataRow label="Team Run">
            <Link
              to={`/team-runs/${task.team_run_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {task.team_run_id.slice(0, 12)}...
            </Link>
          </MetadataRow>
        )}
        {task.reference_url && (
          <MetadataRow label="Reference">
            <a
              href={task.reference_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:text-accent-light transition-colors"
            >
              {task.reference_url}
            </a>
          </MetadataRow>
        )}
        {task.cron_expr && (
          <MetadataRow label="Schedule">
            <span className="font-mono text-xs">{task.cron_expr}</span>
          </MetadataRow>
        )}
      </div>

      {/* Timestamps */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Timing</h3>
        <MetadataRow label="Created">{formatRelativeTime(task.created_at)}</MetadataRow>
        <MetadataRow label="Updated">{formatRelativeTime(task.updated_at)}</MetadataRow>
        {task.next_fire_at && (
          <MetadataRow label="Next Fire">{formatRelativeTime(task.next_fire_at)}</MetadataRow>
        )}
        {task.fired_at && (
          <MetadataRow label="Fired At">{formatRelativeTime(task.fired_at)}</MetadataRow>
        )}
        {task.completed_at && (
          <MetadataRow label="Completed">{formatRelativeTime(task.completed_at)}</MetadataRow>
        )}
      </div>

      {/* Action Config */}
      {task.action_config && task.action_config !== '{}' && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-heading text-sm font-medium">Action Config</h3>
            <CopyButton text={task.action_config} title="Copy action config" />
          </div>
          <pre className="font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {task.action_config}
          </pre>
        </div>
      )}

      {/* Result */}
      {task.result && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-heading text-sm font-medium">Result</h3>
            <CopyButton text={task.result} title="Copy result" />
          </div>
          <pre className="font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {task.result}
          </pre>
        </div>
      )}

      {/* Child Tasks */}
      {children && children.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <h3 className="text-heading text-sm font-medium mb-3">
            Child Tasks ({children.length})
          </h3>
          <div className="space-y-2">
            {children.map((child) => (
              <div key={child.id} className="flex items-center gap-3 py-1.5">
                <TaskStatusBadge status={child.status} />
                <Link
                  to={`/tasks/${child.id}`}
                  className="text-accent text-xs hover:text-accent-light transition-colors"
                >
                  {child.label}
                </Link>
                <div className="flex items-center gap-2 ml-auto">
                  {child.created_by_session && (
                    <Link
                      to={`/sessions/${child.created_by_session}`}
                      className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-accent hover:text-accent-light transition-colors"
                    >
                      SESSION
                    </Link>
                  )}
                  {child.execution_trace_id && (
                    <Link
                      to={`/traces/${child.execution_trace_id}`}
                      className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-accent hover:text-accent-light transition-colors"
                    >
                      TRACE
                    </Link>
                  )}
                  <span className="text-muted/40 text-xs font-mono">
                    {child.action_type}
                  </span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Sessions */}
      {taskSessions && taskSessions.sessions.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <h3 className="text-heading text-sm font-medium mb-3">
            Sessions ({taskSessions.sessions.length})
          </h3>
          <div className="space-y-2">
            {taskSessions.sessions.map((session) => (
              <div key={session.id} className="flex items-center gap-3 py-1.5">
                <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-white/[0.06] text-muted/60">
                  {session.channel_type}
                </span>
                <Link
                  to={`/sessions/${session.id}`}
                  className="text-accent text-xs font-mono hover:text-accent-light transition-colors truncate max-w-[220px]"
                >
                  {session.id}
                </Link>
                {session.task_label && (
                  <span className="text-muted/40 text-xs truncate max-w-[180px]" title={session.task_label}>
                    {session.task_label}
                  </span>
                )}
                <div className="flex items-center gap-3 ml-auto text-muted/40 text-xs shrink-0">
                  <span>{session.message_count} msgs</span>
                  <span>
                    {session.ended_at
                      ? formatDuration(session.started_at, session.ended_at)
                      : 'ongoing'}
                  </span>
                  <span>{formatRelativeTime(session.started_at)}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
