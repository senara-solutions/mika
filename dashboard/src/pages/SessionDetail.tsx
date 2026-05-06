import { useState, Fragment, useMemo } from 'react'
import { useParams, Link, useSearchParams } from 'react-router'
import { useSessionDetail, useSessionMessages, type Message } from '../api/sessions.ts'
import { useTeamRun, useTeamWorkspace, type TeamWorkspaceEntry } from '../api/teams.ts'
import { useSessionLlmCalls } from '../api/llmCalls.ts'
import { useSessionToolCalls, useSessionSkills, useTraceToolCalls } from '../api/toolCalls.ts'
import { CopyButton, Pagination, EmptyState, LoadingState, ErrorState, formatApiError, StatusBadge, formatTimestamp, getAgentColor } from '@senara-solutions/ui'
import type { StatusBadgeVariant } from '@senara-solutions/ui'
import InvestigationPanel, {
  type InvestigationScope,
} from '../components/InvestigationPanel.tsx'
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
  Brain,
  Puzzle,
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
  non_zero_exit?: boolean
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
  traceId,
  metadata,
  onInvestigate,
}: {
  traceId?: string | null
  metadata: string | null
  onInvestigate?: (toolCallIndex: number, toolName: string) => void
}) {
  const { data: apiToolCalls } = useTraceToolCalls(traceId ?? '')
  const metadataToolCalls = parseToolCalls(metadata)

  // Prefer API data (authoritative, no 4KB cap) over metadata (truncated)
  const toolCalls: ToolCall[] = (apiToolCalls && apiToolCalls.length > 0)
    ? apiToolCalls.map(tc => ({
        step: tc.step,
        name: tc.tool_name,
        input_summary: tc.input ?? '',
        output_summary: tc.output ?? '',
        success: tc.success,
        non_zero_exit: tc.non_zero_exit,
      }))
    : metadataToolCalls

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
                      <StatusBadge variant={tc.success ? 'success' : 'error'} label={tc.success ? 'Ok' : 'Fail'} />
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


function formatTokens(n: number | null): string {
  if (n == null) return '-'
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${ms}ms`
}

function llmStatusVariant(status: string): { variant: StatusBadgeVariant; label: string } {
  switch (status) {
    case 'success': return { variant: 'success', label: 'Success' }
    case 'error': return { variant: 'error', label: 'Error' }
    default: return { variant: 'neutral', label: status }
  }
}

function toolSourceBadge(source: string) {
  switch (source) {
    case 'builtin':
      return 'bg-accent/15 text-accent/80'
    case 'skill':
      return 'bg-purple-400/15 text-purple-400'
    case 'mcp':
      return 'bg-amber-400/15 text-amber-400'
    default:
      return 'bg-white/[0.06] text-muted/60'
  }
}

type SessionTab = 'messages' | 'llm-calls' | 'tool-calls' | 'skills'

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

function sessionStatusVariant(status: string): { variant: StatusBadgeVariant; label: string } {
  switch (status) {
    case 'running': return { variant: 'info', label: 'Running' }
    case 'completed': return { variant: 'success', label: 'Completed' }
    case 'failed': return { variant: 'error', label: 'Failed' }
    case 'suspended': return { variant: 'warning', label: 'Suspended' }
    case 'cancelled': return { variant: 'neutral', label: 'Cancelled' }
    default: return { variant: 'neutral', label: status }
  }
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
  const [searchParams, setSearchParams] = useSearchParams()
  const [page, setPage] = useState(1)

  const VALID_SESSION_TABS: SessionTab[] = ['messages', 'llm-calls', 'tool-calls', 'skills']
  const rawTab = searchParams.get('tab')
  const activeTab: SessionTab = VALID_SESSION_TABS.includes(rawTab as SessionTab) ? (rawTab as SessionTab) : 'messages'
  const setActiveTab = (tab: SessionTab) => {
    const next = new URLSearchParams(searchParams)
    if (tab === 'messages') {
      next.delete('tab')
    } else {
      next.set('tab', tab)
    }
    setSearchParams(next)
  }
  const [llmCallsPage, setLlmCallsPage] = useState(1)
  const [toolCallsPage, setToolCallsPage] = useState(1)
  const [expandedToolCallIds, setExpandedToolCallIds] = useState<Set<string>>(new Set())
  const [investigationScope, setInvestigationScope] = useState<InvestigationScope | null>(null)

  const { data: session, isLoading: sessionLoading, isError: sessionError, error: sessionErr, refetch } = useSessionDetail(sessionId ?? '')
  const { data: messages, isLoading: messagesLoading, isError: messagesError, error: messagesErr, refetch: refetchMessages } = useSessionMessages(
    sessionId ?? '',
    page,
  )
  const { data: llmCallsData, isLoading: llmCallsLoading } = useSessionLlmCalls(sessionId ?? '', llmCallsPage)
  const { data: toolCallsData, isLoading: toolCallsLoading } = useSessionToolCalls(sessionId ?? '', toolCallsPage)
  const { data: skills } = useSessionSkills(sessionId ?? '')

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
      return new Date(tsA).getTime() - new Date(tsB).getTime()
    })
    return items
  }, [isTeamSession, workspaceEntries, messages])

  const isLoading = sessionLoading || messagesLoading
  const isError = sessionError || messagesError
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

  const renderOrchestratorCard = (content: string, timestamp: string, label?: string) => {
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
          traceId={msg.trace_id}
          metadata={msg.metadata}
          onInvestigate={(toolCallIndex, toolName) =>
            openInvestigation(msg.id, toolCallIndex, toolName)
          }
        />
      </div>
    )
  }

  const renderRegularMessageCard = (msg: { id: number; role: string; content: string; metadata: string | null; trace_id: string | null; created_at: string }) => {
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
              traceId={msg.trace_id}
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
                <StatusBadge variant="success" label="Active" />
              )}
              {session && session.ended_at && (
                <StatusBadge variant="neutral" label="Ended" />
              )}
              {isTeamSession && (
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-purple-500/15 text-purple-400">
                  <Users size={10} /> Team
                </span>
              )}
              {teamRun && <StatusBadge {...sessionStatusVariant(teamRun.status)} />}
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

      {/* Tabs */}
      <div className="flex items-center gap-1 mb-4 border-b border-white/[0.05]">
        {([
          { key: 'messages' as SessionTab, label: 'Messages', icon: <Bot size={14} />, count: messages?.total },
          { key: 'llm-calls' as SessionTab, label: 'LLM Calls', icon: <Brain size={14} />, count: llmCallsData?.total },
          { key: 'tool-calls' as SessionTab, label: 'Tool Calls', icon: <Wrench size={14} />, count: toolCallsData?.total },
          { key: 'skills' as SessionTab, label: 'Skills', icon: <Puzzle size={14} />, count: skills?.skill_count },
        ]).map((tab) => (
          <button
            key={tab.key}
            onClick={() => setActiveTab(tab.key)}
            className={`flex items-center gap-2 px-4 py-2.5 text-xs font-medium transition-colors border-b-2 -mb-px ${
              activeTab === tab.key
                ? 'border-accent text-accent-light'
                : 'border-transparent text-muted/60 hover:text-muted hover:border-white/[0.1]'
            }`}
          >
            {tab.icon}
            {tab.label}
            {tab.count != null && tab.count > 0 && (
              <span className="text-[10px] text-muted/40 ml-0.5">({tab.count})</span>
            )}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {isLoading ? (
        <LoadingState variant="detail" />
      ) : isError ? (
        <ErrorState message={formatApiError(sessionErr ?? messagesErr)} retry={() => { refetch(); refetchMessages() }} />
      ) : activeTab === 'messages' ? (
        /* Messages tab */
        isTeamSession ? (
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
        )
      ) : activeTab === 'llm-calls' ? (
        /* LLM Calls tab */
        llmCallsLoading ? (
          <LoadingState variant="list" rows={3} />
        ) : !llmCallsData || llmCallsData.data.length === 0 ? (
          <EmptyState message="No LLM calls recorded for this session" />
        ) : (
          <>
            <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                    <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                    <th className="text-left px-4 py-3 font-medium">Provider</th>
                    <th className="text-left px-4 py-3 font-medium">Model</th>
                    <th className="text-right px-4 py-3 font-medium">Input</th>
                    <th className="text-right px-4 py-3 font-medium">Output</th>
                    <th className="text-right px-4 py-3 font-medium">Cache R</th>
                    <th className="text-right px-4 py-3 font-medium">Latency</th>
                    <th className="text-left px-4 py-3 font-medium">Status</th>
                    <th className="text-left px-4 py-3 font-medium">Trace</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/[0.03]">
                  {llmCallsData.data.map((row) => (
                    <tr key={row.id} className="hover:bg-white/[0.02] transition-colors">
                      <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                        {formatTimestamp(row.created_at)}
                      </td>
                      <td className="px-4 py-3 text-xs text-heading font-medium">{row.provider}</td>
                      <td className="px-4 py-3 text-xs font-mono max-w-[200px] truncate">
                        <Link to={`/llm-calls/${row.id}`} className="text-accent hover:text-accent-light transition-colors">
                          {row.model}
                        </Link>
                      </td>
                      <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right">{formatTokens(row.input_tokens)}</td>
                      <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right">{formatTokens(row.output_tokens)}</td>
                      <td className="px-4 py-3 text-xs text-muted/40 font-mono text-right">{formatTokens(row.cache_read_tokens)}</td>
                      <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right whitespace-nowrap">{formatLatency(row.latency_ms)}</td>
                      <td className="px-4 py-3"><StatusBadge {...llmStatusVariant(row.status)} /></td>
                      <td className="px-4 py-3">
                        {row.trace_id ? (
                          <Link
                            to={`/traces/${row.trace_id}`}
                            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                          >
                            {row.trace_id.slice(0, 8)}...
                          </Link>
                        ) : (
                          <span className="text-muted/30 text-xs">-</span>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <Pagination
              page={llmCallsPage}
              perPage={50}
              total={llmCallsData.total}
              onPageChange={setLlmCallsPage}
            />
          </>
        )
      ) : activeTab === 'tool-calls' ? (
        /* Tool Calls tab */
        toolCallsLoading ? (
          <LoadingState variant="list" rows={3} />
        ) : !toolCallsData || toolCallsData.data.length === 0 ? (
          <EmptyState message="No tool calls recorded for this session" />
        ) : (
          <>
            <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                    <th className="w-8 px-2 py-3" />
                    <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                    <th className="text-left px-4 py-3 font-medium">Tool</th>
                    <th className="text-left px-4 py-3 font-medium">Source</th>
                    <th className="text-left px-4 py-3 font-medium">Skill</th>
                    <th className="text-left px-4 py-3 font-medium">Status</th>
                    <th className="text-right px-4 py-3 font-medium">Latency</th>
                    <th className="text-left px-4 py-3 font-medium">Trace</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/[0.03]">
                  {toolCallsData.data.map((row) => {
                    const isOpen = expandedToolCallIds.has(row.id)
                    return (
                      <Fragment key={row.id}>
                        <tr
                          onClick={() => {
                            setExpandedToolCallIds((prev) => {
                              const next = new Set(prev)
                              if (next.has(row.id)) { next.delete(row.id) } else { next.add(row.id) }
                              return next
                            })
                          }}
                          className="hover:bg-white/[0.02] transition-colors cursor-pointer"
                        >
                          <td className="px-2 py-3 text-muted/30">
                            {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          </td>
                          <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                            {formatTimestamp(row.created_at)}
                          </td>
                          <td className="px-4 py-3 text-xs font-mono font-medium max-w-[180px] truncate">
                            <Link
                              to={`/tool-calls/${row.id}`}
                              onClick={(e) => e.stopPropagation()}
                              className="text-accent hover:text-accent-light transition-colors"
                            >
                              {row.tool_name}
                            </Link>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex items-center text-[10px] font-semibold px-2 py-0.5 rounded-full ${toolSourceBadge(row.tool_source)}`}>
                              {row.tool_source}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-xs text-muted/60">
                            {row.skill_name ?? <span className="text-muted/30">-</span>}
                          </td>
                          <td className="px-4 py-3">
                            <StatusBadge variant={row.success ? 'success' : 'error'} label={row.success ? 'Ok' : 'Fail'} />
                          </td>
                          <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right whitespace-nowrap">
                            {formatLatency(row.latency_ms)}
                          </td>
                          <td className="px-4 py-3">
                            {row.trace_id ? (
                              <Link
                                to={`/traces/${row.trace_id}`}
                                onClick={(e) => e.stopPropagation()}
                                className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                              >
                                {row.trace_id.slice(0, 8)}...
                              </Link>
                            ) : (
                              <span className="text-muted/30 text-xs">-</span>
                            )}
                          </td>
                        </tr>
                        {isOpen && (
                          <tr>
                            <td colSpan={8} className="px-6 py-4 bg-white/[0.02]">
                              <div className="space-y-3">
                                {row.input && (
                                  <div>
                                    <div className="flex items-center gap-2">
                                      <span className="text-[10px] text-muted/40 uppercase tracking-wider">Input</span>
                                      <CopyButton text={row.input} title="Copy input" />
                                    </div>
                                    <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                                      {row.input}
                                    </div>
                                  </div>
                                )}
                                {row.output && (
                                  <div>
                                    <div className="flex items-center gap-2">
                                      <span className="text-[10px] text-muted/40 uppercase tracking-wider">Output</span>
                                      <CopyButton text={row.output} title="Copy output" />
                                    </div>
                                    <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                                      {row.output}
                                    </div>
                                  </div>
                                )}
                                {row.error_message && (
                                  <div>
                                    <span className="text-[10px] text-red-400/60 uppercase tracking-wider">Error</span>
                                    <div className="font-mono text-xs text-red-400/80 pl-2 border-l border-red-400/20 mt-1 whitespace-pre-wrap break-all">
                                      {row.error_message}
                                    </div>
                                  </div>
                                )}
                                <div className="text-[10px] text-muted/30">Step {row.step}</div>
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
            <Pagination
              page={toolCallsPage}
              perPage={50}
              total={toolCallsData.total}
              onPageChange={setToolCallsPage}
            />
          </>
        )
      ) : activeTab === 'skills' ? (
        /* Skills tab */
        !skills || skills.loaded_skills.length === 0 ? (
          <EmptyState message="No skills loaded for this session" />
        ) : (
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Skill Name</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {skills.loaded_skills.map((name) => (
                  <tr key={name} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-4 py-3 text-xs text-heading font-mono font-medium">{name}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : null}

      {investigationScope && (
        <InvestigationPanel
          scope={investigationScope}
          onClose={() => setInvestigationScope(null)}
        />
      )}
    </div>
  )
}

