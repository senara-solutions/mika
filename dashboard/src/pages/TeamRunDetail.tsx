import { useState } from 'react'
import { useParams, Link } from 'react-router'
import {
  useTeamRun,
  useTeamRunSummary,
  useTeamWorkspace,
  type TeamWorkspaceEntry,
} from '../api/teams.ts'
import { useTraceLlmCalls, type LlmCallRow } from '../api/llmCalls.ts'
import { useTraceToolCalls, type ToolCallRow } from '../api/toolCalls.ts'
import { TaskStatusBadge, CopyButton, EmptyState, LoadingState, ErrorState, formatApiError, MarkdownContent, formatRelativeTime } from '@senara-solutions/ui'
import TraceIdWidget from '../components/TraceIdWidget.tsx'
import { ChevronDown, ChevronRight, Clock, Cpu, Wrench, Users } from 'lucide-react'

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

function computeDurationMs(startedAt: string, endedAt: string | null): number | null {
  if (!endedAt) return null
  return new Date(endedAt).getTime() - new Date(startedAt).getTime()
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

// ===== Sub-components =====

function StatBadge({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string; size?: number }>
  label: string
  value: string
}) {
  return (
    <div className="flex items-center gap-1.5 text-muted/60 text-xs" title={label}>
      <Icon className="h-3.5 w-3.5" size={14} />
      <span>{value}</span>
    </div>
  )
}

function DarkContainer({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={`bg-bg rounded-xl p-4 border border-white/[0.04] ${className}`}>
      {children}
    </div>
  )
}

/** Agent telemetry summary computed from LLM/tool calls. */
interface AgentTelemetry {
  agentId: string
  llmCallCount: number
  toolCallCount: number
  totalInputTokens: number
  totalOutputTokens: number
  totalCacheReadTokens: number
  totalLatencyMs: number
  toolSuccessCount: number
  models: Map<string, number>
  topTools: Map<string, number>
}

function computeAgentTelemetry(
  llmCalls: LlmCallRow[] | undefined,
  toolCalls: ToolCallRow[] | undefined,
): AgentTelemetry[] {
  const map = new Map<string, AgentTelemetry>()

  const getOrCreate = (agentId: string): AgentTelemetry => {
    if (!map.has(agentId)) {
      map.set(agentId, {
        agentId,
        llmCallCount: 0,
        toolCallCount: 0,
        totalInputTokens: 0,
        totalOutputTokens: 0,
        totalCacheReadTokens: 0,
        totalLatencyMs: 0,
        toolSuccessCount: 0,
        models: new Map(),
        topTools: new Map(),
      })
    }
    return map.get(agentId)!
  }

  if (llmCalls) {
    for (const call of llmCalls) {
      const agent = getOrCreate(call.agent_id)
      agent.llmCallCount++
      agent.totalInputTokens += call.input_tokens
      agent.totalOutputTokens += call.output_tokens
      agent.totalCacheReadTokens += call.cache_read_tokens ?? 0
      agent.totalLatencyMs += call.latency_ms
      agent.models.set(call.model, (agent.models.get(call.model) ?? 0) + 1)
    }
  }

  if (toolCalls) {
    for (const call of toolCalls) {
      const agent = getOrCreate(call.agent_id)
      agent.toolCallCount++
      if (call.success) agent.toolSuccessCount++
      agent.topTools.set(call.tool_name, (agent.topTools.get(call.tool_name) ?? 0) + 1)
    }
  }

  return Array.from(map.values()).sort((a, b) => b.llmCallCount - a.llmCallCount)
}

function AgentTelemetrySection({ agents }: { agents: AgentTelemetry[] }) {
  const [expandedAgent, setExpandedAgent] = useState<string | null>(null)

  if (agents.length === 0) {
    return <EmptyState message="No telemetry data available for this trace" />
  }

  return (
    <div className="space-y-1">
      {agents.map((agent) => {
        const isExpanded = expandedAgent === agent.agentId
        const successRate = agent.toolCallCount > 0
          ? Math.round((agent.toolSuccessCount / agent.toolCallCount) * 100)
          : 0
        const topModels = Array.from(agent.models.entries())
          .sort(([, a], [, b]) => b - a)
          .slice(0, 3)
        const topTools = Array.from(agent.topTools.entries())
          .sort(([, a], [, b]) => b - a)
          .slice(0, 5)

        return (
          <div key={agent.agentId} className="border border-white/[0.04] rounded-lg overflow-hidden">
            <button
              type="button"
              onClick={() => setExpandedAgent(isExpanded ? null : agent.agentId)}
              className="flex w-full items-center gap-3 px-3 py-2.5 text-left hover:bg-white/[0.02] transition-colors"
            >
              {isExpanded ? (
                <ChevronDown size={14} className="text-muted/40 shrink-0" />
              ) : (
                <ChevronRight size={14} className="text-muted/40 shrink-0" />
              )}
              <Link
                to={`/agents/${agent.agentId}`}
                onClick={(e) => e.stopPropagation()}
                className="text-accent text-xs font-medium hover:text-accent-light transition-colors"
              >
                {agent.agentId}
              </Link>
              <div className="flex items-center gap-4 ml-auto text-muted/40 text-xs">
                <span>{agent.llmCallCount} LLM calls</span>
                <span>{agent.toolCallCount} tools ({successRate}%)</span>
                <span>{formatTokens(agent.totalInputTokens + agent.totalOutputTokens)} tokens</span>
              </div>
            </button>

            {isExpanded && (
              <div className="px-3 pb-3 pt-1 border-t border-white/[0.04] space-y-3">
                {/* Models */}
                {topModels.length > 0 && (
                  <div>
                    <h5 className="text-[10px] text-muted/50 uppercase tracking-wider mb-1">Models</h5>
                    <div className="flex flex-wrap gap-1.5">
                      {topModels.map(([model, count]) => (
                        <span key={model} className="text-xs text-muted bg-white/[0.04] rounded px-1.5 py-0.5">
                          {model} <span className="text-muted/40">({count})</span>
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Token breakdown */}
                <div>
                  <h5 className="text-[10px] text-muted/50 uppercase tracking-wider mb-1">Tokens</h5>
                  <div className="grid grid-cols-3 gap-2 text-xs">
                    <div>
                      <span className="text-muted/40">Input: </span>
                      <span className="text-muted">{formatTokens(agent.totalInputTokens)}</span>
                    </div>
                    <div>
                      <span className="text-muted/40">Output: </span>
                      <span className="text-muted">{formatTokens(agent.totalOutputTokens)}</span>
                    </div>
                    <div>
                      <span className="text-muted/40">Cache read: </span>
                      <span className="text-muted">{formatTokens(agent.totalCacheReadTokens)}</span>
                    </div>
                  </div>
                </div>

                {/* Latency */}
                <div>
                  <h5 className="text-[10px] text-muted/50 uppercase tracking-wider mb-1">Latency</h5>
                  <span className="text-xs text-muted">{formatDurationMs(agent.totalLatencyMs)} total</span>
                  {agent.llmCallCount > 0 && (
                    <span className="text-xs text-muted/40 ml-2">
                      (avg {formatDurationMs(Math.round(agent.totalLatencyMs / agent.llmCallCount))})
                    </span>
                  )}
                </div>

                {/* Top tools */}
                {topTools.length > 0 && (
                  <div>
                    <h5 className="text-[10px] text-muted/50 uppercase tracking-wider mb-1">Top Tools</h5>
                    <div className="space-y-0.5">
                      {topTools.map(([tool, count]) => (
                        <div key={tool} className="flex items-center justify-between text-xs">
                          <span className="text-muted font-mono">{tool}</span>
                          <span className="text-muted/40">{count}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        )
      })}
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

// ===== Main Page =====

export default function TeamRunDetail() {
  const { runId } = useParams()
  const { data: run, isLoading: runLoading, error: runError, refetch } = useTeamRun(runId)
  const { data: summary } = useTeamRunSummary(runId)
  const { data: workspace } = useTeamWorkspace(runId)

  // Fetch telemetry when trace_id is available
  const traceId = run?.trace_id ?? ''
  const { data: llmCalls } = useTraceLlmCalls(traceId)
  const { data: toolCalls } = useTraceToolCalls(traceId)

  if (runLoading) {
    return <LoadingState variant="detail" />
  }

  if (runError) {
    return <ErrorState message={formatApiError(runError)} retry={() => refetch()} />
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

  // Compute telemetry aggregates
  const agentTelemetry = computeAgentTelemetry(llmCalls, toolCalls)
  const durationMs = computeDurationMs(run.started_at, run.ended_at)
  const totalLlmCalls = llmCalls?.length ?? 0
  const totalToolCalls = toolCalls?.length ?? 0
  const totalTokens = llmCalls?.reduce((sum, c) => sum + c.input_tokens + c.output_tokens, 0) ?? 0

  // Iteration indicator logic
  const capturedIterations = displayIterations.length
  const iterationLabel = capturedIterations < run.max_iterations && capturedIterations < run.iteration
    ? `Iteration ${run.iteration}/${run.max_iterations} (${capturedIterations} captured)`
    : `Iteration ${run.iteration}/${run.max_iterations}`

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
            <div>{iterationLabel}</div>
          </div>
        </div>

        {/* Trace ID Widget */}
        {run.trace_id && (
          <div className="flex items-center gap-4 mt-3 pt-3 border-t border-white/[0.05]">
            <TraceIdWidget traceId={run.trace_id} callCount={totalLlmCalls} />
          </div>
        )}

        {/* Stats row */}
        {(durationMs !== null || totalLlmCalls > 0 || totalToolCalls > 0) && (
          <div className="flex items-center gap-5 mt-3 pt-3 border-t border-white/[0.05]">
            {durationMs !== null && (
              <StatBadge icon={Clock} label="Duration" value={formatDurationMs(durationMs)} />
            )}
            {totalLlmCalls > 0 && (
              <StatBadge icon={Cpu} label="LLM Calls" value={`${totalLlmCalls} calls`} />
            )}
            {totalToolCalls > 0 && (
              <StatBadge icon={Wrench} label="Tool Calls" value={`${totalToolCalls} tools`} />
            )}
            {agentTelemetry.length > 0 && (
              <StatBadge icon={Users} label="Agents" value={`${agentTelemetry.length} agents`} />
            )}
            {totalTokens > 0 && (
              <span className="text-muted/40 text-xs">{formatTokens(totalTokens)} tokens</span>
            )}
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

      {/* Agent Telemetry */}
      {run.trace_id && agentTelemetry.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-xl p-5 mb-5">
          <h3 className="text-heading text-base font-semibold mb-3">Agent Telemetry</h3>
          <AgentTelemetrySection agents={agentTelemetry} />
        </div>
      )}

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
