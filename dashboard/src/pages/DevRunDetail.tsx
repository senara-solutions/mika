import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useDevRun, useMergeDevRun } from '../api/devRuns.ts'
import { TaskStatusBadge, CopyButton, formatRelativeTime } from '@senara-solutions/ui'

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

function MetadataRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className="text-muted/60 text-xs w-28 shrink-0 uppercase tracking-wider">{label}</span>
      <span className="text-heading text-sm">{children}</span>
    </div>
  )
}

export default function DevRunDetail() {
  const { taskId } = useParams<{ taskId: string }>()
  const { data: run, isLoading, error } = useDevRun(taskId)
  const mergeMutation = useMergeDevRun()
  const [showConfirm, setShowConfirm] = useState(false)

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

  const canMerge = run.pr_url && run.status === 'in_progress'

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
            <CopyButton text={run.id} label="Copy ID" />
          </div>
        </div>
        {canMerge && (
          <div>
            {showConfirm ? (
              <div className="flex items-center gap-2">
                <span className="text-muted/60 text-xs">Merge PR?</span>
                <button
                  onClick={() => {
                    mergeMutation.mutate(run.id)
                    setShowConfirm(false)
                  }}
                  disabled={mergeMutation.isPending}
                  className="px-3 py-1.5 bg-green-600 hover:bg-green-500 text-white text-xs rounded-lg transition-colors disabled:opacity-50"
                >
                  {mergeMutation.isPending ? 'Merging...' : 'Confirm'}
                </button>
                <button
                  onClick={() => setShowConfirm(false)}
                  className="px-3 py-1.5 bg-white/[0.06] hover:bg-white/[0.1] text-muted text-xs rounded-lg transition-colors"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => setShowConfirm(true)}
                className="px-4 py-2 bg-accent/20 hover:bg-accent/30 text-accent text-sm rounded-lg transition-colors"
              >
                Merge PR
              </button>
            )}
          </div>
        )}
      </div>

      {mergeMutation.isSuccess && (
        <div className="bg-green-900/30 border border-green-500/30 text-green-300 px-4 py-3 rounded-lg text-sm mb-4">
          PR merged successfully.
        </div>
      )}
      {mergeMutation.isError && (
        <div className="bg-red-900/30 border border-red-500/30 text-red-300 px-4 py-3 rounded-lg text-sm mb-4">
          Merge failed: {mergeMutation.error?.message}
        </div>
      )}

      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Run Details</h3>
        <MetadataRow label="ID">
          <span className="font-mono text-xs">{run.id}</span>
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
          {run.pr_url ? (
            <a
              href={run.pr_url}
              target="_blank"
              rel="noopener noreferrer"
              className="text-accent hover:text-accent-light transition-colors"
            >
              #{run.pr_number}
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
