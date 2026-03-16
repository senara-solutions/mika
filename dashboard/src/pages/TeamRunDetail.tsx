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
import { formatRelativeTime } from '../utils/formatTime.ts'
import { ChevronDown, ChevronRight } from 'lucide-react'

const ENTRY_TYPE_STYLES: Record<string, string> = {
  goal: 'bg-purple-500/10 text-purple-400',
  orchestrator: 'bg-blue-500/10 text-blue-400',
  assignment: 'bg-teal-500/10 text-teal-400',
  agent_response: 'bg-green-500/10 text-green-400',
  critic: 'bg-orange-500/10 text-orange-400',
  deliverable: 'bg-emerald-500/10 text-emerald-400',
  error: 'bg-red-500/10 text-red-400',
}

function EntryTypeBadge({ type }: { type: string }) {
  const style = ENTRY_TYPE_STYLES[type] ?? 'bg-gray-500/10 text-gray-400'
  return (
    <span className={`inline-flex px-2 py-0.5 rounded-full text-xs font-medium ${style}`}>
      {type}
    </span>
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
              <div className="space-y-1">
                {assignments.map((a) => (
                  <div
                    key={a.id}
                    className="bg-white/[0.02] rounded-lg px-3 py-2 text-xs text-muted"
                  >
                    <span className="text-heading font-medium">{a.agent_name ?? 'unknown'}</span>
                    {' — '}
                    <span>{a.content.slice(0, 200)}</span>
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
                        {ar.agent_name}
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
                  {c.content}
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
        <div className="flex items-center gap-4 mt-3 pt-3 border-t border-white/[0.05]">
          {run.trace_id && (
            <Link
              to={`/traces/${run.trace_id}`}
              className="text-accent text-xs hover:text-accent-light transition-colors"
            >
              View Trace &rarr;
            </Link>
          )}
        </div>

        {/* Goal */}
        <div className="mt-3 pt-3 border-t border-white/[0.05]">
          <h4 className="text-xs text-muted/60 uppercase tracking-wider mb-1">Goal</h4>
          <p className="text-sm text-muted">{run.goal}</p>
        </div>

        {/* Deliverable */}
        {run.deliverable && (
          <div className="mt-3 pt-3 border-t border-white/[0.05]">
            <h4 className="text-xs text-muted/60 uppercase tracking-wider mb-1">Deliverable</h4>
            <p className="text-sm text-muted whitespace-pre-wrap">{run.deliverable}</p>
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
      {iterations.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
          <h3 className="text-heading text-base font-semibold mb-4">Iteration Timeline</h3>
          {iterations.map(([iteration, entries], idx) => (
            <IterationSection
              key={iteration}
              iteration={iteration}
              entries={entries}
              runId={run.id}
              defaultOpen={idx === iterations.length - 1}
            />
          ))}
        </div>
      )}

      {/* Workspace Entries */}
      {workspace && workspace.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
          <h3 className="text-heading text-base font-semibold mb-3">
            Workspace Entries
            <span className="ml-2 text-xs bg-accent/10 text-accent px-2 py-0.5 rounded-full font-medium">
              {workspace.length}
            </span>
          </h3>
          <div className="overflow-hidden rounded-lg border border-white/[0.05]">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-2 font-medium">Type</th>
                  <th className="text-left px-4 py-2 font-medium">Agent</th>
                  <th className="text-left px-4 py-2 font-medium">Iter</th>
                  <th className="text-left px-4 py-2 font-medium">Content</th>
                  <th className="text-left px-4 py-2 font-medium">Time</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {workspace.map((entry) => (
                  <tr key={entry.id} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-4 py-2">
                      <EntryTypeBadge type={entry.entry_type} />
                    </td>
                    <td className="px-4 py-2 text-xs text-muted">
                      {entry.agent_name ?? '\u2014'}
                    </td>
                    <td className="px-4 py-2 text-xs text-muted">{entry.iteration}</td>
                    <td className="px-4 py-2 text-xs text-muted max-w-[400px] truncate">
                      {entry.content.slice(0, 100)}
                    </td>
                    <td className="px-4 py-2 text-xs text-muted/70 font-mono">
                      {formatRelativeTime(entry.created_at)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  )
}
