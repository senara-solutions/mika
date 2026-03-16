import { useState } from 'react'
import { useParams, Link } from 'react-router'
import {
  useTeamRun,
  useTeamRunSummary,
  useTeamWorkspace,
  type TeamWorkspaceEntry,
} from '../api/teams.ts'
import TaskStatusBadge from '../components/TaskStatusBadge.tsx'
import CopyButton from '../components/CopyButton.tsx'
import EmptyState from '../components/EmptyState.tsx'
import MarkdownContent from '../components/MarkdownContent.tsx'
import { formatRelativeTime } from '../utils/formatTime.ts'
import { ChevronDown, ChevronRight } from 'lucide-react'

function DarkContainer({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={`bg-bg rounded-xl p-4 border border-white/[0.04] ${className}`}>
      {children}
    </div>
  )
}

function IterationSection({
  iteration,
  entries,
  runId,
  defaultOpen,
}: {
  iteration: number
  entries: TeamWorkspaceEntry[]
  runId: string
  defaultOpen: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)

  const assignments = entries.filter((e) => e.entry_type === 'assignment')
  const agentResponses = entries.filter((e) => e.entry_type === 'agent_response')
  const criticEntries = entries.filter((e) => e.entry_type === 'critic')

  return (
    <div className="border-l-2 border-white/[0.08] ml-3 pl-4 mb-4">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 mb-2 group w-full text-left"
      >
        <span className="w-6 h-6 rounded-full bg-accent/20 text-accent text-xs font-bold flex items-center justify-center -ml-7">
          {iteration}
        </span>
        {open ? (
          <ChevronDown size={14} className="text-muted/60" />
        ) : (
          <ChevronRight size={14} className="text-muted/60" />
        )}
        <span className="text-heading text-sm font-medium">Iteration {iteration}</span>
        {criticEntries.length > 0 && (
          <span className="text-xs text-muted/60">
            — {criticEntries[0].content.includes('approved') ? 'Approved' : 'Reviewed'}
          </span>
        )}
      </button>

      {open && (
        <div className="space-y-3">
          {/* Assign phase */}
          {assignments.length > 0 && (
            <div>
              <h5 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Phase: Assign
              </h5>
              <div className="space-y-2">
                {assignments.map((a) => (
                  <div
                    key={a.id}
                    className="bg-bg rounded-lg px-3 py-2 border border-white/[0.04]"
                  >
                    <div className="text-xs text-muted break-words">
                      <span className="text-heading font-medium">@{a.agent_name ?? 'unknown'}</span>
                      <MarkdownContent content={a.content} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Execute phase */}
          {agentResponses.length > 0 && (
            <div>
              <h5 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Phase: Execute
              </h5>
              <div className="space-y-2">
                {agentResponses.map((ar) => (
                  <div
                    key={ar.id}
                    className="bg-white/[0.02] border border-white/[0.05] rounded-lg px-3 py-2"
                  >
                    <div className="flex items-center justify-between mb-1">
                      <Link
                        to={`/agents/${ar.agent_name}`}
                        className="text-accent text-xs font-medium hover:text-accent-light transition-colors"
                      >
                        @{ar.agent_name}
                      </Link>
                      <Link
                        to={`/sessions/team-${runId}-${ar.agent_name}`}
                        className="text-accent text-xs hover:text-accent-light transition-colors"
                      >
                        View Session &rarr;
                      </Link>
                    </div>
                    <p className="text-xs text-muted line-clamp-3">{ar.content.slice(0, 300)}</p>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Review phase */}
          {criticEntries.length > 0 && (
            <div>
              <h5 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Phase: Review
              </h5>
              {criticEntries.map((c) => (
                <div
                  key={c.id}
                  className="bg-orange-500/5 border border-orange-500/10 rounded-lg px-3 py-2 text-xs text-muted"
                >
                  <MarkdownContent content={c.content} />
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export default function TeamRunDetail() {
  const { runId } = useParams()
  const { data: run, isLoading: runLoading, error: runError } = useTeamRun(runId)
  const { data: summary } = useTeamRunSummary(runId)
  const { data: workspace } = useTeamWorkspace(runId)

  if (runLoading) {
    return <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
  }

  if (runError) {
    return (
      <div className="text-red-400 py-8 text-center text-sm">
        Error: {runError instanceof Error ? runError.message : 'Unknown error'}
      </div>
    )
  }

  if (!run) {
    return <EmptyState message="Team run not found" />
  }

  // Group workspace entries by iteration
  const iterationMap = new Map<number, TeamWorkspaceEntry[]>()
  if (workspace) {
    for (const entry of workspace) {
      const existing = iterationMap.get(entry.iteration) ?? []
      existing.push(entry)
      iterationMap.set(entry.iteration, existing)
    }
  }
  const iterations = Array.from(iterationMap.entries()).sort(([a], [b]) => a - b)
  const displayIterations = iterations.filter(([, entries]) =>
    entries.some((e) => ['assignment', 'agent_response', 'critic'].includes(e.entry_type))
  )

  return (
    <div>
      {/* Breadcrumb */}
      <div className="text-xs text-muted/60 mb-3">
        <Link to="/team-runs" className="hover:text-muted transition-colors">
          Team Runs
        </Link>
        {' > '}
        <span className="text-muted">{run.team_name}</span>
        {' > '}
        <span className="text-muted font-mono">{run.id.slice(0, 16)}...</span>
      </div>

      {/* Header */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-heading text-xl font-semibold">{run.team_name}</h2>
            <div className="flex items-center gap-3 mt-2">
              <span className="text-xs font-mono text-muted/70">{run.id}</span>
              <CopyButton text={run.id} />
              <TaskStatusBadge status={run.status} />
            </div>
          </div>
          <div className="text-right text-xs text-muted/60 space-y-1">
            <div>Started {formatRelativeTime(run.started_at)}</div>
            {run.ended_at && <div>Ended {formatRelativeTime(run.ended_at)}</div>}
            <div>
              Iteration {run.iteration}/{run.max_iterations}
            </div>
          </div>
        </div>

        {/* Links */}
        {run.trace_id && (
          <div className="flex items-center gap-4 mt-3 pt-3 border-t border-white/[0.05]">
            <Link
              to={`/traces/${run.trace_id}`}
              className="text-accent text-xs hover:text-accent-light transition-colors"
            >
              View Trace &rarr;
            </Link>
          </div>
        )}

        {/* Goal */}
        <div className="mt-3 pt-3 border-t border-white/[0.05]">
          <div className="flex items-center gap-2 mb-1">
            <h4 className="text-xs text-muted/60 uppercase tracking-wider">Goal</h4>
            <CopyButton text={run.goal} />
          </div>
          <DarkContainer className="max-h-48 overflow-y-auto">
            <p className="text-sm text-muted whitespace-pre-wrap break-words">{run.goal}</p>
          </DarkContainer>
        </div>

        {/* Deliverable */}
        {run.deliverable && (
          <div className="mt-3 pt-3 border-t border-white/[0.05]">
            <div className="flex items-center gap-2 mb-1">
              <h4 className="text-xs text-muted/60 uppercase tracking-wider">Deliverable</h4>
              <CopyButton text={run.deliverable} />
            </div>
            <DarkContainer className="max-h-96 overflow-y-auto">
              <MarkdownContent content={run.deliverable} />
            </DarkContainer>
          </div>
        )}

        {/* Failure */}
        {run.failure_reason && (
          <div className="mt-3 pt-3 border-t border-white/[0.05]">
            <h4 className="text-xs text-red-400/60 uppercase tracking-wider mb-1">
              Failure Reason
            </h4>
            <p className="text-sm text-red-400">{run.failure_reason}</p>
          </div>
        )}
      </div>

      {/* Summary: Agent Results & Critic */}
      {summary && (
        <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
          <h3 className="text-heading text-base font-semibold mb-3">Run Summary</h3>

          {summary.agent_results.length > 0 && (
            <div className="mb-3">
              <h4 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Agent Results
              </h4>
              <div className="space-y-2">
                {summary.agent_results.map((ar) => (
                  <div key={ar.agent_name} className="bg-white/[0.02] rounded-lg px-3 py-2">
                    <div className="flex items-center justify-between mb-1">
                      <Link
                        to={`/agents/${ar.agent_name}`}
                        className="text-accent text-xs font-medium hover:text-accent-light transition-colors"
                      >
                        {ar.agent_name}
                      </Link>
                      <Link
                        to={`/sessions/team-${run.id}-${ar.agent_name}`}
                        className="text-accent text-xs hover:text-accent-light transition-colors"
                      >
                        Session &rarr;
                      </Link>
                    </div>
                    <p className="text-xs text-muted">{ar.response_preview}</p>
                  </div>
                ))}
              </div>
            </div>
          )}

          {summary.task_statuses.length > 0 && (
            <div className="mb-3">
              <h4 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Task Statuses
              </h4>
              <div className="space-y-1">
                {summary.task_statuses.map((ts) => (
                  <div key={ts.task_id} className="flex items-center gap-2 text-xs">
                    <TaskStatusBadge status={ts.status} />
                    <span className="text-muted">{ts.agent_id}: {ts.label}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {summary.critic_feedback && (
            <div>
              <h4 className="text-xs text-muted/60 uppercase tracking-wider mb-2">
                Critic Feedback
              </h4>
              <div className="bg-orange-500/5 border border-orange-500/10 rounded-lg px-3 py-2 text-xs text-muted">
                {summary.critic_feedback}
              </div>
            </div>
          )}
        </div>
      )}

      {/* Iteration Timeline */}
      {displayIterations.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
          <h3 className="text-heading text-base font-semibold mb-4">Iteration Timeline</h3>
          {displayIterations.map(([iterNum, entries], idx) => (
            <IterationSection
              key={iterNum}
              iteration={iterNum}
              entries={entries}
              runId={run.id}
              defaultOpen={idx === displayIterations.length - 1}
            />
          ))}
        </div>
      )}

    </div>
  )
}
