import { useParams, Link } from 'react-router'
import { useDevRun } from '../api/devRuns.ts'
import { TaskStatusBadge, CopyButton, formatRelativeTime } from '@senara-solutions/ui'
import { MetadataRow } from '../components/MetadataRow.tsx'

function formatDuration(ms: number | null): string {
  if (ms === null || ms === undefined) return '—'
  const totalSecs = Math.floor(ms / 1000)
  const mins = Math.floor(totalSecs / 60)
  const secs = totalSecs % 60
  if (mins === 0) return `${secs}s`
  return `${mins}m ${secs}s`
}

function formatCost(usd: number | null): string {
  if (usd === null || usd === undefined) return '—'
  return `$${usd.toFixed(2)}`
}

export default function DevRunDetail() {
  const { taskId } = useParams<{ taskId: string }>()
  const { data: run, isLoading, error } = useDevRun(taskId)

  if (isLoading) {
    return <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
  }
  if (error) {
    return (
      <div className="text-red-400 py-8 text-center text-sm">
        Error: {error instanceof Error ? error.message : 'Unknown error'}
      </div>
    )
  }
  if (!run) {
    return <div className="text-muted/60 py-8 text-center text-sm">Dev run not found</div>
  }

  return (
    <div>
      <div className="mb-5">
        <Link to="/dev-runs" className="text-muted/60 text-xs hover:text-muted transition-colors">
          &larr; Back to Dev Runs
        </Link>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h2 className="text-heading text-xl font-semibold">{run.label}</h2>
          <div className="flex items-center gap-3 mt-2">
            <TaskStatusBadge status={run.status} />
            <span className="text-muted/60 text-xs">{run.agent_id}</span>
            <CopyButton text={run.id} title="Copy ID" />
          </div>
        </div>
      </div>

      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Run Details</h3>
        <MetadataRow label="ID">
          <Link
            to={`/tasks/${run.id}`}
            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
          >
            {run.id}
          </Link>
        </MetadataRow>
        <MetadataRow label="Created">{formatRelativeTime(run.created_at)}</MetadataRow>
        <MetadataRow label="Updated">{formatRelativeTime(run.updated_at)}</MetadataRow>
        {run.completed_at && (
          <MetadataRow label="Completed">{formatRelativeTime(run.completed_at)}</MetadataRow>
        )}
        {run.reference_url && (
          <MetadataRow label="Issue">
            <a
              href={run.reference_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:text-accent-light transition-colors"
            >
              {run.reference_url}
            </a>
          </MetadataRow>
        )}
      </div>

      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Claude Pilot Metadata</h3>
        <MetadataRow label="Branch">
          <span className="font-mono text-xs">{run.branch ?? '—'}</span>
        </MetadataRow>
        <MetadataRow label="Repo">
          <span className="font-mono text-xs">{run.repo ?? '—'}</span>
        </MetadataRow>
        <MetadataRow label="PR">
          {run.pr_url && run.pr_number ? (
            <a
              href={run.pr_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:text-accent-light transition-colors"
            >
              #{run.pr_number}
            </a>
          ) : run.pr_url ? (
            <a
              href={run.pr_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:text-accent-light transition-colors"
            >
              {run.pr_url}
            </a>
          ) : (
            '—'
          )}
        </MetadataRow>
        <MetadataRow label="Session ID">
          <span className="font-mono text-xs">{run.session_id ?? '—'}</span>
        </MetadataRow>
        <MetadataRow label="Cost">{formatCost(run.cost_usd)}</MetadataRow>
        <MetadataRow label="Duration">{formatDuration(run.duration_ms)}</MetadataRow>
        <MetadataRow label="Turns">{run.turns ?? '—'}</MetadataRow>
      </div>
    </div>
  )
}
