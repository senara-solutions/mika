import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useDevRun } from '../api/devRuns.ts'
import { useGitHubIssue, useGitHubPull, type GitHubReview } from '../api/github.ts'
import { useTaskSessions, useTaskChildren } from '../api/tasks.ts'
import { useSessionMessages, type Message } from '../api/sessions.ts'
import {
  TaskStatusBadge,
  CopyButton,
  MarkdownContent,
  EmptyState,
  LoadingState,
  ErrorState,
  formatApiError,
  formatRelativeTime,
} from '@senara-solutions/ui'
import { MetadataRow } from '../components/MetadataRow.tsx'
import { CollapsibleCard } from '../components/CollapsibleCard.tsx'
import { PipelineTimeline, type PipelineStep } from '../components/PipelineTimeline.tsx'
import { parseGitHubUrl } from '../utils/github.ts'
import {
  Clock,
  DollarSign,
  GitPullRequest,
  MessageSquare,
  ChevronDown,
  FileCode,
  CheckCircle2,
  XCircle,
  MessageCircle,
} from 'lucide-react'

// ===== Helpers =====

function formatDurationMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const totalSecs = Math.floor(ms / 1000)
  if (totalSecs < 60) return `${totalSecs}s`
  const mins = Math.floor(totalSecs / 60)
  const secs = totalSecs % 60
  if (mins < 60) return `${mins}m ${secs}s`
  const hours = Math.floor(mins / 60)
  const remMins = mins % 60
  return `${hours}h ${remMins}m`
}

function formatDuration(ms: number | null): string {
  if (ms === null || ms === undefined) return '--'
  return formatDurationMs(ms)
}

function formatCost(usd: number | null): string {
  if (usd === null || usd === undefined) return '--'
  return `$${usd.toFixed(2)}`
}

/** Strip common prefixes like "[self_dev]" from task labels for a cleaner title. */
function cleanLabel(label: string): string {
  return label.replace(/^\[.*?\]\s*/, '')
}

/** Derive pipeline steps from dev run metadata and PR reviews. */
function derivePipelineSteps(
  run: {
    status: string
    branch: string | null
    pr_url: string | null
  },
  hasReviews: boolean,
  reviewVerdict: string | null,
): PipelineStep[] {
  const hasBranch = !!run.branch
  const hasPr = !!run.pr_url
  const isTerminal = ['completed', 'failed', 'cancelled'].includes(run.status)

  // Determine how far the pipeline got
  const planDone = true // if we have a dev run, plan started
  const workDone = hasBranch
  const prDone = hasPr
  const qaDone = hasReviews && reviewVerdict !== null

  function stepStatus(done: boolean, prevDone: boolean): 'completed' | 'active' | 'pending' {
    if (done) return 'completed'
    if (prevDone && !isTerminal) return 'active'
    return 'pending'
  }

  return [
    { label: 'Plan', status: stepStatus(planDone, true) },
    { label: 'Work', status: stepStatus(workDone, planDone) },
    { label: 'PR', status: stepStatus(prDone, workDone) },
    { label: 'QA', status: stepStatus(qaDone, prDone) },
  ]
}

// ===== Sub-components =====

function StatBadge({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>
  label: string
  value: string
}) {
  return (
    <div className="flex items-center gap-1.5 text-muted/60 text-xs" title={label}>
      <Icon className="h-3.5 w-3.5" />
      <span>{value}</span>
    </div>
  )
}

function ReviewVerdictIcon({ state }: { state: string }) {
  if (state === 'APPROVED') {
    return <CheckCircle2 className="h-4 w-4 text-emerald-400" />
  }
  if (state === 'CHANGES_REQUESTED') {
    return <XCircle className="h-4 w-4 text-red-400" />
  }
  return <MessageCircle className="h-4 w-4 text-yellow-400" />
}

function ReviewCard({ review }: { review: GitHubReview }) {
  const stateLabel =
    review.state === 'APPROVED'
      ? 'Approved'
      : review.state === 'CHANGES_REQUESTED'
        ? 'Changes Requested'
        : 'Commented'

  const stateColor =
    review.state === 'APPROVED'
      ? 'text-emerald-400'
      : review.state === 'CHANGES_REQUESTED'
        ? 'text-red-400'
        : 'text-yellow-400'

  return (
    <div className="flex items-start gap-3 py-2">
      <ReviewVerdictIcon state={review.state} />
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-heading">{review.user}</span>
          <span className={`text-xs font-medium ${stateColor}`}>{stateLabel}</span>
          {review.submitted_at && (
            <span className="text-muted/40 text-xs">{formatRelativeTime(review.submitted_at)}</span>
          )}
        </div>
        {review.body && (
          <div className="mt-1 text-xs text-muted/60">
            <MarkdownContent content={review.body} />
          </div>
        )}
      </div>
    </div>
  )
}

/** Expandable session row with lazy-loaded messages. */
function SessionRow({ sessionId, label, messageCount, startedAt, endedAt }: {
  sessionId: string
  label: string | null
  messageCount: number
  startedAt: string
  endedAt: string | null
}) {
  const [expanded, setExpanded] = useState(false)

  return (
    <div className="border-t border-white/[0.03] first:border-t-0">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-3 py-2.5 text-left"
      >
        <ChevronDown
          className={`h-3.5 w-3.5 text-muted/40 transition-transform ${expanded ? 'rotate-0' : '-rotate-90'}`}
        />
        <Link
          to={`/sessions/${sessionId}`}
          onClick={(e) => e.stopPropagation()}
          className="text-accent text-xs font-mono hover:text-accent-light transition-colors truncate max-w-[200px]"
        >
          {sessionId}
        </Link>
        {label && (
          <span className="text-muted/40 text-xs truncate max-w-[200px]" title={label}>
            {label}
          </span>
        )}
        <div className="flex items-center gap-3 ml-auto text-muted/40 text-xs shrink-0">
          <span>{messageCount} msgs</span>
          <span>
            {endedAt ? formatDurationFromDates(startedAt, endedAt) : 'ongoing'}
          </span>
        </div>
      </button>
      {expanded && <SessionMessages sessionId={sessionId} />}
    </div>
  )
}

function formatDurationFromDates(start: string, end: string): string {
  return formatDurationMs(new Date(end).getTime() - new Date(start).getTime())
}

/** Lazy-loaded session messages (only fetched when expanded). */
function SessionMessages({ sessionId }: { sessionId: string }) {
  const { data, isLoading } = useSessionMessages(sessionId, 1, 50)

  if (isLoading) {
    return <div className="text-muted/40 text-xs py-2 pl-8">Loading messages...</div>
  }

  if (!data || data.data.length === 0) {
    return <div className="text-muted/40 text-xs py-2 pl-8">No messages</div>
  }

  return (
    <div className="pl-8 pb-3 space-y-2 max-h-96 overflow-y-auto">
      {data.data.map((msg: Message) => (
        <div key={msg.id} className="flex gap-2">
          <span
            className={`text-[10px] font-medium px-1.5 py-0.5 rounded shrink-0 ${
              msg.role === 'user'
                ? 'bg-blue-500/10 text-blue-400'
                : msg.role === 'assistant'
                  ? 'bg-purple-500/10 text-purple-400'
                  : 'bg-white/[0.04] text-muted/60'
            }`}
          >
            {msg.role}
          </span>
          <span className="text-xs text-muted/60 line-clamp-3 break-all">
            {msg.content.slice(0, 500)}
            {msg.content.length > 500 && '...'}
          </span>
        </div>
      ))}
      {data.total > 50 && (
        <Link
          to={`/sessions/${sessionId}`}
          className="text-accent text-xs hover:text-accent-light transition-colors"
        >
          View all {data.total} messages &rarr;
        </Link>
      )}
    </div>
  )
}

// ===== Main Component =====

export default function DevRunDetail() {
  const { taskId } = useParams<{ taskId: string }>()
  const { data: run, isLoading, error, refetch } = useDevRun(taskId)

  // Parse GitHub references from the run data
  const issueRef = run?.reference_url ? parseGitHubUrl(run.reference_url) : null
  const prRef = run?.pr_url
    ? parseGitHubUrl(run.pr_url)
    : run?.repo && run?.pr_number
      ? { owner: run.repo.split('/')[0], repo: run.repo.split('/')[1], number: run.pr_number, type: 'pull' as const }
      : null

  // Fetch GitHub data (conditional on refs existing)
  const { data: issue, error: issueError, isLoading: issueLoading, refetch: refetchIssue } = useGitHubIssue(
    issueRef?.owner ?? null,
    issueRef?.repo ?? null,
    issueRef?.number ?? null,
  )
  const { data: pull, error: pullError, isLoading: pullLoading, refetch: refetchPull } = useGitHubPull(
    prRef?.owner ?? null,
    prRef?.repo ?? null,
    prRef?.number ?? null,
  )

  // Fetch sessions and children
  const { data: taskSessions } = useTaskSessions(taskId)
  const { data: children } = useTaskChildren(taskId)

  if (isLoading) {
    return <LoadingState variant="detail" />
  }
  if (error) {
    return <ErrorState message={formatApiError(error)} retry={() => refetch()} />
  }
  if (!run) {
    return <EmptyState message="Dev run not found" />
  }

  const hasReviews = (pull?.reviews?.length ?? 0) > 0
  const latestReview = hasReviews ? pull!.reviews[pull!.reviews.length - 1] : null
  const reviewVerdict = latestReview?.state ?? null

  const pipelineSteps = derivePipelineSteps(
    { status: run.status, branch: run.branch, pr_url: run.pr_url },
    hasReviews,
    reviewVerdict,
  )

  return (
    <div>
      {/* Back navigation */}
      <div className="mb-5">
        <Link to="/dev-runs" className="text-muted/60 text-xs hover:text-muted transition-colors">
          &larr; Back to Dev Runs
        </Link>
      </div>

      {/* ===== Run Header ===== */}
      <div className="flex items-start justify-between mb-6">
        <div>
          <h2 className="text-heading text-xl font-semibold">{cleanLabel(run.label)}</h2>
          <div className="flex items-center gap-3 mt-2">
            <TaskStatusBadge status={run.status} />
            <span className="text-muted/60 text-xs">{run.agent_id}</span>
            <CopyButton text={run.id} title="Copy ID" />
          </div>
        </div>
      </div>

      {/* Stats row */}
      <div className="flex items-center gap-5 mb-5">
        <StatBadge icon={DollarSign} label="Cost" value={formatCost(run.cost_usd)} />
        <StatBadge icon={Clock} label="Duration" value={formatDuration(run.duration_ms)} />
        <StatBadge icon={MessageSquare} label="Turns" value={String(run.turns ?? '--')} />
        {pull && (
          <StatBadge
            icon={FileCode}
            label="Files changed"
            value={String(pull.changed_files)}
          />
        )}
        {run.reference_url && (
          <a
            href={run.reference_url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-accent text-xs hover:text-accent-light transition-colors"
          >
            {issueRef ? `#${issueRef.number}` : 'Issue'}
          </a>
        )}
        {run.pr_url && (
          <a
            href={run.pr_url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-accent text-xs hover:text-accent-light transition-colors flex items-center gap-1"
          >
            <GitPullRequest className="h-3 w-3" />
            {run.pr_number ? `#${run.pr_number}` : 'PR'}
          </a>
        )}
      </div>

      {/* ===== Issue Card ===== */}
      {issueRef && (
        <div className="mb-4">
          <CollapsibleCard
            title="Issue"
            badge={
              issue && (
                <span className="text-muted/40 text-xs">
                  #{issueRef.number}
                </span>
              )
            }
          >
            {issueLoading && (
              <div className="text-muted/40 text-xs animate-pulse">Loading issue...</div>
            )}
            {issueError && (
              <ErrorState
                variant="detail-section"
                message={formatApiError(issueError)}
                retry={() => refetchIssue()}
              />
            )}
            {issue && (
              <div>
                <h4 className="text-heading text-sm font-medium mb-2">{issue.title}</h4>
                {issue.labels.length > 0 && (
                  <div className="flex gap-1.5 mb-3">
                    {issue.labels.map((l) => (
                      <span
                        key={l.name}
                        className="text-[10px] px-1.5 py-0.5 rounded-full border border-white/[0.08]"
                        style={{ color: `#${l.color}` }}
                      >
                        {l.name}
                      </span>
                    ))}
                  </div>
                )}
                {issue.body ? (
                  <div className="text-xs text-muted/70 max-h-96 overflow-y-auto">
                    <MarkdownContent content={issue.body} />
                  </div>
                ) : (
                  <div className="text-muted/40 text-xs">No description</div>
                )}
              </div>
            )}
          </CollapsibleCard>
        </div>
      )}
      {!issueRef && !run.reference_url && (
        <div className="mb-4">
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
            <EmptyState message="No linked issue" />
          </div>
        </div>
      )}

      {/* ===== Pipeline Timeline ===== */}
      <div className="mb-4">
        <PipelineTimeline steps={pipelineSteps} />
      </div>

      {/* ===== PR Summary Card ===== */}
      {prRef && (
        <div className="mb-4">
          <CollapsibleCard
            title="Pull Request"
            badge={
              run.pr_number && (
                <a
                  href={run.pr_url ?? '#'}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent text-xs hover:text-accent-light transition-colors"
                  onClick={(e) => e.stopPropagation()}
                >
                  #{run.pr_number}
                </a>
              )
            }
          >
            {pullLoading && (
              <div className="text-muted/40 text-xs animate-pulse">Loading PR...</div>
            )}
            {pullError && (
              <ErrorState
                variant="detail-section"
                message={formatApiError(pullError)}
                retry={() => refetchPull()}
              />
            )}
            {pull && (
              <div>
                <h4 className="text-heading text-sm font-medium mb-2">{pull.title}</h4>
                <div className="flex items-center gap-3 mb-3 text-xs">
                  <span className="text-emerald-400">+{pull.additions}</span>
                  <span className="text-red-400">-{pull.deletions}</span>
                  <span className="text-muted/40">{pull.changed_files} files</span>
                  <span
                    className={`px-1.5 py-0.5 rounded-full text-[10px] font-medium ${
                      pull.merged
                        ? 'bg-purple-500/10 text-purple-400'
                        : pull.state === 'open'
                          ? 'bg-emerald-500/10 text-emerald-400'
                          : 'bg-red-500/10 text-red-400'
                    }`}
                  >
                    {pull.merged ? 'Merged' : pull.state}
                  </span>
                </div>
                {pull.body ? (
                  <div className="text-xs text-muted/70 max-h-64 overflow-y-auto">
                    <MarkdownContent content={pull.body} />
                  </div>
                ) : (
                  <div className="text-muted/40 text-xs">No description</div>
                )}
              </div>
            )}
          </CollapsibleCard>
        </div>
      )}
      {!prRef && (
        <div className="mb-4">
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
            <EmptyState message="No PR yet" />
          </div>
        </div>
      )}

      {/* ===== Agent Activity (Sessions) ===== */}
      {taskSessions && taskSessions.sessions.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mb-4">
          <h3 className="text-heading text-sm font-medium mb-3">
            Agent Activity ({taskSessions.sessions.length} session{taskSessions.sessions.length !== 1 && 's'})
          </h3>
          <div>
            {taskSessions.sessions.map((session) => (
              <SessionRow
                key={session.id}
                sessionId={session.id}
                label={session.task_label}
                messageCount={session.message_count}
                startedAt={session.started_at}
                endedAt={session.ended_at}
              />
            ))}
          </div>
        </div>
      )}

      {/* ===== Child Tasks ===== */}
      {children && children.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mb-4">
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

      {/* ===== QA Verdict ===== */}
      {pull && pull.reviews.length > 0 && (
        <div className="mb-4">
          <CollapsibleCard title="QA Verdict">
            <div className="space-y-1">
              {pull.reviews.map((review, i) => (
                <ReviewCard key={`${review.user}-${i}`} review={review} />
              ))}
            </div>
          </CollapsibleCard>
        </div>
      )}

      {/* ===== Claude Pilot Metadata (collapsed) ===== */}
      <CollapsibleCard title="Claude Pilot Metadata" defaultOpen={false}>
        <div className="space-y-1">
          <MetadataRow label="Task ID">
            <Link
              to={`/tasks/${run.id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {run.id}
            </Link>
          </MetadataRow>
          <MetadataRow label="Branch">
            <span className="font-mono text-xs">{run.branch ?? '--'}</span>
          </MetadataRow>
          <MetadataRow label="Repo">
            <span className="font-mono text-xs">{run.repo ?? '--'}</span>
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
            ) : (
              '--'
            )}
          </MetadataRow>
          <MetadataRow label="Session ID">
            {run.session_id ? (
              <Link
                to={`/sessions/${run.session_id}`}
                className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
              >
                {run.session_id}
              </Link>
            ) : (
              <span className="font-mono text-xs">--</span>
            )}
          </MetadataRow>
          <MetadataRow label="Cost">{formatCost(run.cost_usd)}</MetadataRow>
          <MetadataRow label="Duration">{formatDuration(run.duration_ms)}</MetadataRow>
          <MetadataRow label="Turns">{run.turns ?? '--'}</MetadataRow>
          <MetadataRow label="Created">{formatRelativeTime(run.created_at)}</MetadataRow>
          <MetadataRow label="Updated">{formatRelativeTime(run.updated_at)}</MetadataRow>
          {run.completed_at && (
            <MetadataRow label="Completed">{formatRelativeTime(run.completed_at)}</MetadataRow>
          )}
        </div>
      </CollapsibleCard>
    </div>
  )
}
