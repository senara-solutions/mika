import { useState, Fragment, useMemo } from 'react'
import { useParams, Link } from 'react-router'
import { useSessionDetail, useSessionMessages, type Message } from '../api/sessions.ts'
import { useTeamRun, useTeamWorkspace, type TeamWorkspaceEntry } from '../api/teams.ts'
import CopyButton from '../components/CopyButton.tsx'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import InvestigationPanel, {
  type InvestigationScope,
} from '../components/InvestigationPanel.tsx'
import { formatTimestamp } from '../utils/formatTime.ts'
import { getAgentColor } from '../utils/agentColors.ts'
import {
  ArrowLeft,
  User,
  Bot,
  Settings,
  Wrench,
  ChevronRight,
  ChevronDown,
  Search,
  Users,
  Target,
} from 'lucide-react'

type TimelineItem =
  | { kind: 'workspace'; entry: TeamWorkspaceEntry }
  | { kind: 'message'; msg: Message }

interface ToolCall {
  step: number
  name: string
  input_summary: string
  output_summary: string
  success: boolean
}

function parseToolCalls(metadata: string | null): ToolCall[] {
  if (!metadata) return []
  try {
    const parsed = JSON.parse(metadata)
    return Array.isArray(parsed.tool_calls) ? parsed.tool_calls : []
  } catch {
    return []
  }
}

function getAgentNameFromMetadata(metadata: string | null): string | null {
  if (!metadata) return null
  try {
    const parsed = JSON.parse(metadata)
    return parsed.agent_name ?? null
  } catch {
    return null
  }
}

const QUICK_COPY_KEYS: Record<string, string> = {
  run_shell: 'command',
  read_workspace: 'path',
  read_agent_file: 'path',
  write_agent_file: 'path',
  list_agent_files: 'path',
  // backward compat: old tool names in historical data
  read_home_file: 'path',
  write_file: 'path',
  list_home_files: 'path',
  search_memory: 'query',
  create_agent: 'name',
  delegate_task: 'task',
  store_fact: 'name',
  update_fact: 'name',
  create_reminder: 'label',
  create_skill: 'name',
  delete_skill: 'name',
  web_search: 'query',
  read_file: 'path',
  run_team: 'goal',
  get_documentation: 'topic',
}

function extractQuickCopy(toolName: string, inputSummary: string): string | null {
  const key = QUICK_COPY_KEYS[toolName]
  if (!key) return null
  try {
    const parsed = JSON.parse(inputSummary)
    const value = parsed[key]
    return typeof value === 'string' ? value : null
  } catch {
    return null
  }
}

function truncateText(text: string, maxLen = 80): string {
  if (text.length <= maxLen) return text
  // Strip backend's trailing "..." before re-truncating to avoid "text......"
  const cleaned = text.endsWith('...') ? text.slice(0, -3) : text
  if (cleaned.length <= maxLen) return text
  return cleaned.slice(0, maxLen) + '...'
}

function ToolCallsTable({
  metadata,
  onInvestigate,
}: {
  metadata: string | null
  onInvestigate?: (toolCallIndex: number, toolName: string) => void
}) {
  const toolCalls = parseToolCalls(metadata)
  const [expanded, setExpanded] = useState<Set<number>>(new Set())

  if (toolCalls.length === 0) return null

  const toggleExpand = (index: number) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(index)) { next.delete(index) } else { next.add(index) }
      return next
    })
  }

  return (
    <div className="mt-3 pl-8">
      <div className="bg-white/[0.03] border border-white/[0.06] rounded-xl overflow-x-auto">
        {/* Header */}
        <div className="flex items-center gap-2 px-3 py-2 border-b border-white/[0.06]">
          <Wrench size={12} className="text-muted/40" />
          <span className="text-[10px] text-muted/40 uppercase tracking-wider">
            {toolCalls.length} tool call{toolCalls.length !== 1 ? 's' : ''}
          </span>
        </div>

        {/* Table */}
        <table className="w-full text-xs">
          <thead>
            <tr className="border-b border-white/[0.05] text-muted/40 text-[10px] uppercase tracking-wider">
              <th className="w-6 px-2 py-2" />
              <th className="w-12 px-2 py-2" />
              <th className="text-left px-2 py-2 font-medium">Tool</th>
              <th className="text-left px-2 py-2 font-medium">Input</th>
              <th className="text-left px-2 py-2 font-medium">Output</th>
              {onInvestigate && <th className="w-8 px-2 py-2" />}
            </tr>
          </thead>
          <tbody className="divide-y divide-white/[0.03]">
            {toolCalls.map((tc, i) => {
              const isOpen = expanded.has(i)
              return (
                <Fragment key={i}>
                  <tr
                    onClick={() => toggleExpand(i)}
                    className="hover:bg-white/[0.02] transition-colors cursor-pointer"
                  >
                    {/* Chevron */}
                    <td className="px-2 py-2 text-muted/30">
                      {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                    </td>
                    {/* Status */}
                    <td className="px-2 py-2">
                      {tc.success ? (
                        <span className="inline-flex items-center gap-1 text-emerald-400">
                          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                          <span className="text-[10px]">ok</span>
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-red-400">
                          <span className="w-1.5 h-1.5 rounded-full bg-red-400" />
                          <span className="text-[10px]">fail</span>
                        </span>
                      )}
                    </td>
                    {/* Tool name */}
                    <td className="px-2 py-2 font-mono text-heading font-medium max-w-[160px] truncate">
                      {tc.name}
                    </td>
                    {/* Input */}
                    <td className="px-2 py-2 font-mono text-muted/60 max-w-[200px]">
                      <div className="flex items-center gap-1.5">
                        {tc.input_summary ? (() => {
                          const qk = QUICK_COPY_KEYS[tc.name]
                          const qv = qk ? extractQuickCopy(tc.name, tc.input_summary) : null
                          return qv ? (
                            <>
                              <span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-accent/15 text-accent/60 font-medium">
                                {qk}
                              </span>
                              <span className="truncate">{qv}</span>
                              <CopyButton text={qv} title={`Copy ${qk}`} />
                            </>
                          ) : (
                            <>
                              <span className="truncate">{truncateText(tc.input_summary)}</span>
                              <CopyButton text={tc.input_summary} title="Copy input" />
                            </>
                          )
                        })() : (
                          <span className="text-muted/30">&mdash;</span>
                        )}
                      </div>
                    </td>
                    {/* Output */}
                    <td className="px-2 py-2 font-mono text-muted/60 max-w-[240px]">
                      <div className="flex items-center gap-1">
                        <span className="truncate">
                          {tc.output_summary ? (
                            truncateText(tc.output_summary)
                          ) : (
                            <span className="text-muted/30">&mdash;</span>
                          )}
                        </span>
                        {tc.output_summary && <CopyButton text={tc.output_summary} />}
                      </div>
                    </td>
                    {onInvestigate && (
                      <td className="px-2 py-2">
                        <button
                          onClick={(e) => {
                            e.stopPropagation()
                            onInvestigate(i, tc.name)
                          }}
                          className="p-1 rounded hover:bg-accent/10 text-muted/30 hover:text-accent transition-colors"
                          title="Investigate this tool call"
                        >
                          <Search size={12} />
                        </button>
                      </td>
                    )}
                  </tr>
                  {/* Expanded detail row */}
                  {isOpen && (
                    <tr>
                      <td colSpan={onInvestigate ? 6 : 5} className="px-4 py-3 bg-white/[0.02]">
                        <div className="space-y-2">
                          {tc.input_summary && (
                            <div>
                              <div className="flex items-center gap-2">
                                <span className="text-[10px] text-muted/40 uppercase tracking-wider">Input</span>
                                <CopyButton text={tc.input_summary} title="Copy all" />
                              </div>
                              <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all">
                                {tc.input_summary}
                              </div>
                            </div>
                          )}
                          {tc.output_summary && (
                            <div>
                              <div className="flex items-center gap-2">
                                <span className="text-[10px] text-muted/40 uppercase tracking-wider">Output</span>
                                <CopyButton text={tc.output_summary} title="Copy all" />
                              </div>
                              <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all">
                                {tc.output_summary}
                              </div>
                            </div>
                          )}
                          <div className="text-[10px] text-muted/30">Step {tc.step}</div>
                        </div>
                      </td>
                    </tr>
                  )}
                </Fragment>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}


function AgentAvatar({ name }: { name: string }) {
  const color = getAgentColor(name)
  return (
    <div
      className={`w-8 h-8 rounded-full flex items-center justify-center text-xs font-bold ${color.bg} ${color.text}`}
      title={name}
    >
      {name.charAt(0).toUpperCase()}
    </div>
  )
}

function statusBadge(status: string) {
  const styles: Record<string, string> = {
    running: 'bg-blue-500/15 text-blue-400',
    completed: 'bg-emerald-500/15 text-emerald-400',
    failed: 'bg-red-500/15 text-red-400',
    suspended: 'bg-amber-500/15 text-amber-400',
    cancelled: 'bg-white/[0.06] text-muted/60',
  }
  return (
    <span className={`inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase ${styles[status] ?? styles.cancelled}`}>
      {status}
    </span>
  )
}

function roleConfig(role: string) {
  switch (role) {
    case 'user':
      return {
        bg: 'bg-accent/8 border-accent/15',
        icon: <User size={14} />,
        iconBg: 'bg-accent/15 text-accent',
        label: 'text-accent-light',
        name: 'User',
        align: 'mr-12',
      }
    case 'assistant':
      return {
        bg: 'bg-bg-card border-white/[0.06]',
        icon: <Bot size={14} />,
        iconBg: 'bg-emerald-400/15 text-emerald-400',
        label: 'text-heading',
        name: 'Assistant',
        align: 'ml-12',
      }
    case 'system':
    case 'summary':
      return {
        bg: 'bg-white/[0.02] border-white/[0.04]',
        icon: <Settings size={14} />,
        iconBg: 'bg-white/[0.06] text-muted/60',
        label: 'text-muted/60',
        name: role.charAt(0).toUpperCase() + role.slice(1),
        align: 'mx-16',
      }
    case 'tool_result':
      return {
        bg: 'bg-amber-400/5 border-amber-400/15',
        icon: <Wrench size={14} />,
        iconBg: 'bg-amber-400/15 text-amber-400',
        label: 'text-amber-400',
        name: 'Tool Result',
        align: 'ml-12',
      }
    default:
      return {
        bg: 'bg-bg-card border-white/[0.05]',
        icon: <User size={14} />,
        iconBg: 'bg-white/[0.06] text-muted',
        label: 'text-muted',
        name: role,
        align: '',
      }
  }
}

export default function SessionDetail() {
  const { sessionId } = useParams<{ sessionId: string }>()
  const [page, setPage] = useState(1)
  const [investigationScope, setInvestigationScope] = useState<InvestigationScope | null>(null)

  const { data: session, isLoading: sessionLoading } = useSessionDetail(sessionId ?? '')
  const { data: messages, isLoading: messagesLoading } = useSessionMessages(
    sessionId ?? '',
    page,
  )

  const isTeamSession = session?.channel_type === 'team'
  // Team session IDs follow the pattern "team-{run_id}"
  const teamRunId = isTeamSession && sessionId ? sessionId.replace(/^team-/, '') : undefined
  const { data: teamRun } = useTeamRun(teamRunId)
  const { data: workspaceEntries } = useTeamWorkspace(teamRunId)

  // Collect unique agent names from workspace for the header avatar row
  const teamAgentNames = useMemo(() => {
    if (!workspaceEntries) return [] as string[]
    const seen = new Set<string>()
    const names: string[] = []
    for (const e of workspaceEntries) {
      if (e.entry_type === 'assignment' && e.agent_name && !seen.has(e.agent_name)) {
        seen.add(e.agent_name)
        names.push(e.agent_name)
      }
    }
    return names
  }, [workspaceEntries])

  // Get goal text from workspace entries
  const goalText = useMemo(() => {
    if (!workspaceEntries) return null
    const goal = workspaceEntries.find((e) => e.entry_type === 'goal')
    return goal?.content ?? null
  }, [workspaceEntries])

  // Build unified timeline merging workspace entries + messages for team sessions
  const teamTimeline = useMemo((): TimelineItem[] => {
    if (!isTeamSession) return []
    const items: TimelineItem[] = []
    if (workspaceEntries) {
      for (const entry of workspaceEntries) {
        items.push({ kind: 'workspace', entry })
      }
    }
    if (messages?.data) {
      for (const msg of messages.data) {
        items.push({ kind: 'message', msg })
      }
    }
    items.sort((a, b) => {
      const tsA = a.kind === 'workspace' ? a.entry.created_at : a.msg.created_at
      const tsB = b.kind === 'workspace' ? b.entry.created_at : b.msg.created_at
      return tsA - tsB
    })
    return items
  }, [isTeamSession, workspaceEntries, messages])

  const isLoading = sessionLoading || messagesLoading

  const openInvestigation = (
    messageId: number,
    toolCallIndex?: number,
    toolName?: string,
  ) => {
    setInvestigationScope({
      type: toolCallIndex != null ? 'tool_call' : 'message',
      messageId,
      toolCallIndex,
      toolName,
      sessionId: sessionId ?? '',
      agentId: session?.agent_id ?? '',
    })
  }

  const openSessionInvestigation = () => {
    // Find the last assistant message as a proxy for full-session investigation
    const allMessages = messages?.data ?? []
    const lastAssistant = [...allMessages].reverse().find((m: Message) => m.role === 'assistant')
    if (!lastAssistant) return
    setInvestigationScope({
      type: 'session',
      messageId: lastAssistant.id,
      sessionId: sessionId ?? '',
      agentId: session?.agent_id ?? '',
    })
  }

  const orchestratorName = session?.agent_id
    ? session.agent_id.charAt(0).toUpperCase() + session.agent_id.slice(1)
    : 'Orchestrator'

  const renderOrchestratorCard = (content: string, timestamp: number, label?: string) => {
    return (
      <div className="ml-12">
        <div className="border rounded-xl p-4 bg-bg-card border-white/[0.06]">
          <div className="flex items-center gap-2 mb-2">
            <div className="w-6 h-6 rounded-md flex items-center justify-center bg-violet-400/15 text-violet-400">
              <Users size={14} />
            </div>
            <span className="text-xs font-semibold text-violet-300">{orchestratorName}</span>
            {label && <span className="text-[10px] text-muted/40 uppercase tracking-wider">{label}</span>}
            <span className="text-[10px] text-muted/30 ml-auto font-mono">{formatTimestamp(timestamp)}</span>
          </div>
          <div className="text-sm text-muted/80 whitespace-pre-wrap break-words pl-8">{content}</div>
        </div>
      </div>
    )
  }

  const formatOrchestratorPlan = (content: string): string => {
    try {
      const parsed = JSON.parse(content)
      if (Array.isArray(parsed)) {
        const lines = parsed.map((item: { agent: string; task: string }) =>
          `\u2022 ${item.agent} \u2192 ${item.task}`
        )
        return `Here's the plan:\n${lines.join('\n')}`
      }
      return content
    } catch {
      return content
    }
  }

  const renderWorkspaceEntry = (entry: TeamWorkspaceEntry) => {
    switch (entry.entry_type) {
      case 'goal':
        // Skip — goal bar at top is sufficient
        return null
      case 'orchestrator':
        return renderOrchestratorCard(formatOrchestratorPlan(entry.content), entry.created_at, 'Plan')
      case 'assignment': {
        const agentMention = entry.agent_name ? `@${entry.agent_name}: ` : ''
        const mentionColor = entry.agent_name ? getAgentColor(entry.agent_name) : null
        return (
          <div className="ml-12">
            <div className="border rounded-xl p-4 bg-bg-card border-white/[0.06]">
              <div className="flex items-center gap-2 mb-2">
                <div className="w-6 h-6 rounded-md flex items-center justify-center bg-violet-400/15 text-violet-400">
                  <Users size={14} />
                </div>
                <span className="text-xs font-semibold text-violet-300">{orchestratorName}</span>
                <span className="text-[10px] text-muted/40 uppercase tracking-wider">Assignment</span>
                <span className="text-[10px] text-muted/30 ml-auto font-mono">{formatTimestamp(entry.created_at)}</span>
              </div>
              <div className="text-sm text-muted/80 whitespace-pre-wrap break-words pl-8">
                {mentionColor && (
                  <span className={`font-semibold ${mentionColor.text}`}>{agentMention}</span>
                )}
                {entry.content}
              </div>
            </div>
          </div>
        )
      }
      case 'critic':
        return renderOrchestratorCard(
          entry.content,
          entry.created_at,
          `Feedback · Iteration ${entry.iteration}`,
        )
      case 'final_deliverable':
        return renderOrchestratorCard(entry.content, entry.created_at, 'Deliverable')
      default:
        // error or unknown entry types — render as orchestrator message
        return renderOrchestratorCard(entry.content, entry.created_at, entry.entry_type)
    }
  }

  const renderTeamMessageCard = (msg: Message) => {
    const agentName = getAgentNameFromMetadata(msg.metadata)

    // Non-agent messages (user, system, tool_result) render with standard config
    if (!agentName || msg.role !== 'assistant') {
      const config = roleConfig(msg.role)
      return (
        <div className={`border rounded-xl p-4 ${config.bg}`}>
          <div className="flex items-center gap-2 mb-2">
            <div className={`w-6 h-6 rounded-md flex items-center justify-center ${config.iconBg}`}>
              {config.icon}
            </div>
            <span className={`text-xs font-semibold ${config.label}`}>
              {config.name}
            </span>
            <span className="text-[10px] text-muted/30 ml-auto font-mono">
              {formatTimestamp(msg.created_at)}
            </span>
          </div>
          <div className="text-sm text-muted/80 whitespace-pre-wrap break-words max-h-96 overflow-y-auto pl-8">
            {msg.content}
          </div>
        </div>
      )
    }

    // Agent card with colored accent
    const color = getAgentColor(agentName)

    return (
      <div className={`border rounded-xl p-4 ${color.bg} border-current/10`} style={{ borderColor: `color-mix(in srgb, currentColor 10%, transparent)` }}>
        {/* Agent header */}
        <div className="flex items-center gap-3 mb-2">
          <AgentAvatar name={agentName} />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className={`text-sm font-semibold ${color.text}`}>{agentName}</span>
              <span className="text-[10px] text-muted/30 font-mono">
                {formatTimestamp(msg.created_at)}
              </span>
              <CopyButton text={msg.content} title="Copy response" className="ml-1" />
              <button
                onClick={() => openInvestigation(msg.id)}
                className="p-1 rounded hover:bg-accent/10 text-muted/30 hover:text-accent transition-colors"
                title="Investigate this message"
              >
                <Search size={12} />
              </button>
            </div>
          </div>
        </div>

        {/* Message content */}
        <div className="text-sm text-muted/80 whitespace-pre-wrap break-words max-h-96 overflow-y-auto pl-11">
          {msg.content}
        </div>

        {/* Tool calls */}
        <ToolCallsTable
          metadata={msg.metadata}
          onInvestigate={(toolCallIndex, toolName) =>
            openInvestigation(msg.id, toolCallIndex, toolName)
          }
        />
      </div>
    )
  }

  const renderRegularMessageCard = (msg: { id: number; role: string; content: string; metadata: string | null; created_at: number }) => {
    const config = roleConfig(msg.role)
    return (
      <div key={msg.id} className={config.align}>
        <div className={`border rounded-xl p-4 ${config.bg}`}>
          <div className="flex items-center gap-2 mb-2">
            <div className={`w-6 h-6 rounded-md flex items-center justify-center ${config.iconBg}`}>
              {config.icon}
            </div>
            <span className={`text-xs font-semibold ${config.label}`}>
              {msg.role === 'assistant' && session?.agent_id
                ? session.agent_id.charAt(0).toUpperCase() + session.agent_id.slice(1)
                : config.name}
            </span>
            {msg.role === 'assistant' && (
              <>
                <CopyButton text={msg.content} title="Copy response" className="ml-2" />
                <button
                  onClick={() => openInvestigation(msg.id)}
                  className="p-1 rounded hover:bg-accent/10 text-muted/30 hover:text-accent transition-colors"
                  title="Investigate this message"
                >
                  <Search size={12} />
                </button>
              </>
            )}
            <span className="text-[10px] text-muted/30 ml-auto font-mono">
              {formatTimestamp(msg.created_at)}
            </span>
          </div>
          <div className="text-sm text-muted/80 whitespace-pre-wrap break-words max-h-96 overflow-y-auto pl-8">
            {msg.content}
          </div>
          {msg.role === 'assistant' && (
            <ToolCallsTable
              metadata={msg.metadata}
              onInvestigate={(toolCallIndex, toolName) =>
                openInvestigation(msg.id, toolCallIndex, toolName)
              }
            />
          )}
        </div>
      </div>
    )
  }

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Link
            to="/sessions"
            className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
          >
            <ArrowLeft size={18} />
          </Link>
          <div>
            <div className="flex items-center gap-3">
              <h2 className="text-heading text-lg font-semibold font-mono">{sessionId}</h2>
              <CopyButton text={sessionId ?? ''} className="ml-1" />
              {session && !session.ended_at && (
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-emerald-500/15 text-emerald-400">
                  Active
                </span>
              )}
              {session && session.ended_at && (
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-white/[0.06] text-muted/60">
                  Ended
                </span>
              )}
              {isTeamSession && (
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-purple-500/15 text-purple-400">
                  <Users size={10} /> Team
                </span>
              )}
              {teamRun && statusBadge(teamRun.status)}
            </div>
            {session && (
              <div className="flex items-center gap-3 mt-1">
                <span className="text-xs text-muted/60">
                  {isTeamSession && teamRun
                    ? `${teamRun.team_name} \u00b7 ${formatTimestamp(session.started_at)}`
                    : `Session initialized via ${session.channel_type} \u00b7 ${formatTimestamp(session.started_at)}`}
                </span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Goal bar (team sessions only) */}
      {isTeamSession && goalText && (
        <div className="flex items-center gap-4 mb-5 px-4 py-3 bg-bg-card border border-white/[0.05] rounded-xl">
          <Target size={14} className="text-muted/40 shrink-0" />
          <p className="text-xs text-muted/80 flex-1 min-w-0 truncate">
            <span className="font-medium text-muted/60">Goal:</span> {goalText}
          </p>
          {teamRun && (
            <span className="text-[10px] text-muted/40 shrink-0 font-mono">
              {teamRun.iteration}/{teamRun.max_iterations}
            </span>
          )}
          {teamAgentNames.length > 0 && (
            <div className="flex items-center -space-x-1.5 shrink-0">
              {teamAgentNames.map((name) => {
                const c = getAgentColor(name)
                return (
                  <div
                    key={name}
                    className={`w-6 h-6 rounded-full flex items-center justify-center text-[9px] font-bold ring-2 ring-bg-card ${c.bg} ${c.text}`}
                    title={name}
                  >
                    {name.charAt(0).toUpperCase()}
                  </div>
                )
              })}
            </div>
          )}
        </div>
      )}

      {/* Session info bar (non-team sessions) */}
      {session && !isTeamSession && (
        <div className="flex items-center gap-4 mb-5 px-4 py-2.5 bg-bg-card border border-white/[0.05] rounded-xl">
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-muted/40 uppercase tracking-wider">Agent</span>
            <span className="text-xs text-heading font-medium">{session.agent_id}</span>
          </div>
          <span className="text-white/[0.1]">|</span>
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-muted/40 uppercase tracking-wider">Channel</span>
            <span className="text-xs text-muted bg-white/[0.04] px-1.5 py-0.5 rounded">
              {session.channel_type}
            </span>
          </div>
          <span className="text-white/[0.1]">|</span>
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-muted/40 uppercase tracking-wider">Messages</span>
            <span className="text-xs text-heading font-medium">{messages?.total ?? '...'}</span>
          </div>
          <div className="ml-auto">
            <button
              onClick={openSessionInvestigation}
              disabled={!messages?.data?.some((m) => m.role === 'assistant')}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-accent/70 hover:text-accent hover:bg-accent/10 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
              title="Investigate full session"
            >
              <Search size={12} />
              Investigate
            </button>
          </div>
        </div>
      )}

      {/* Content area — single column for both team and regular */}
      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : isTeamSession ? (
        teamTimeline.length === 0 ? (
          <EmptyState message="No messages in this session" />
        ) : (
          <>
            <div className="space-y-3">
              {teamTimeline.map((item) => {
                if (item.kind === 'workspace') {
                  const rendered = renderWorkspaceEntry(item.entry)
                  return rendered ? <div key={`ws-${item.entry.id}`}>{rendered}</div> : null
                }
                return <div key={`msg-${item.msg.id}`}>{renderTeamMessageCard(item.msg)}</div>
              })}
            </div>
            <Pagination
              page={page}
              perPage={50}
              total={messages?.total ?? 0}
              onPageChange={setPage}
            />
          </>
        )
      ) : !messages || messages.data.length === 0 ? (
        <EmptyState message="No messages in this session" />
      ) : (
        <>
          <div className="space-y-3">
            {messages.data.map((msg) => renderRegularMessageCard(msg))}
          </div>
          <Pagination
            page={page}
            perPage={50}
            total={messages.total}
            onPageChange={setPage}
          />
        </>
      )}

      {investigationScope && (
        <InvestigationPanel
          scope={investigationScope}
          onClose={() => setInvestigationScope(null)}
        />
      )}
    </div>
  )
}

