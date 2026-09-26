import { useState, Fragment } from 'react'
import { useParams, Link } from 'react-router'
import { useTraceDetail, useTraceMessages, type TraceMessage } from '../api/timeline.ts'
import { useTraceLlmCalls } from '../api/llmCalls.ts'
import { useTraceToolCalls } from '../api/toolCalls.ts'
import { CopyButton, EmptyState, LoadingState, ErrorState, formatApiError, StatusBadge, formatTimestamp, eventTypeBadge, eventTypeColor } from '@samidarko/ui'
import type { StatusBadgeVariant } from '@samidarko/ui'
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
  Brain,
} from 'lucide-react'

// ===== Shared helpers (mirrors SessionDetail patterns) =====

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
  const cleaned = text.endsWith('...') ? text.slice(0, -3) : text
  if (cleaned.length <= maxLen) return text
  return cleaned.slice(0, maxLen) + '...'
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
        <div className="flex items-center gap-2 px-3 py-2 border-b border-white/[0.06]">
          <Wrench size={12} className="text-muted/40" />
          <span className="text-[10px] text-muted/40 uppercase tracking-wider">
            {toolCalls.length} tool call{toolCalls.length !== 1 ? 's' : ''}
          </span>
        </div>
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
                    <td className="px-2 py-2 text-muted/30">
                      {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                    </td>
                    <td className="px-2 py-2">
                      <StatusBadge variant={tc.success ? 'success' : 'error'} label={tc.success ? 'Ok' : 'Fail'} />
                    </td>
                    <td className="px-2 py-2 font-mono text-heading font-medium max-w-[160px] truncate">
                      {tc.name}
                    </td>
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

// ===== Message card renderer =====

function MessageCard({
  msg,
  onInvestigate,
}: {
  msg: TraceMessage
  onInvestigate: (messageId: number, toolCallIndex?: number, toolName?: string) => void
}) {
  const config = roleConfig(msg.role)
  return (
    <div className={config.align}>
      <div className={`border rounded-xl p-4 ${config.bg}`}>
        <div className="flex items-center gap-2 mb-2">
          <div className={`w-6 h-6 rounded-md flex items-center justify-center ${config.iconBg}`}>
            {config.icon}
          </div>
          <span className={`text-xs font-semibold ${config.label}`}>
            {config.name}
          </span>
          {msg.role === 'assistant' && (
            <button
              onClick={() => onInvestigate(msg.id)}
              className="ml-2 p-1 rounded hover:bg-accent/10 text-muted/30 hover:text-accent transition-colors"
              title="Investigate this message"
            >
              <Search size={12} />
            </button>
          )}
          <span className="text-[10px] text-muted/40 uppercase tracking-wider ml-1">Agent</span>
          <span className="text-xs text-heading font-medium">{msg.agent_id}</span>
          <span className="text-white/[0.06] mx-1">|</span>
          <span className="text-[10px] text-muted/40 uppercase tracking-wider">Session</span>
          <Link
            to={`/sessions/${msg.session_id}`}
            className="text-xs text-accent font-mono hover:text-accent-light transition-colors"
          >
            {msg.session_id.slice(0, 8)}...
          </Link>
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
              onInvestigate(msg.id, toolCallIndex, toolName)
            }
          />
        )}
      </div>
    </div>
  )
}

// ===== Non-message event card (audit, task) =====

function EventCard({ event }: { event: { event_type: string; event_subtype: string; agent_id: string | null; session_id: string | null; summary: string | null; created_at: string } }) {
  const badge = eventTypeBadge(event.event_type)
  return (
    <div className={`border rounded-xl p-4 ${eventTypeColor(event.event_type)}`}>
      <div className="flex items-center gap-3 mb-2">
        <span
          className={`inline-flex items-center gap-1.5 text-xs font-semibold px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
        >
          {event.event_type}
        </span>
        <span className="text-xs text-muted font-mono">{event.event_subtype}</span>
        <span className="text-xs text-muted/50 ml-auto font-mono">
          {formatTimestamp(event.created_at)}
        </span>
      </div>
      <div className="flex items-center gap-2 mb-2">
        <span className="text-[10px] text-muted/40 uppercase tracking-wider">Agent</span>
        <span className="text-xs text-heading font-medium">{event.agent_id ?? '—'}</span>
        {event.session_id && (
          <>
            <span className="text-white/[0.06] mx-1">|</span>
            <span className="text-[10px] text-muted/40 uppercase tracking-wider">Session</span>
            <Link
              to={`/sessions/${event.session_id}`}
              className="text-xs text-accent font-mono hover:text-accent-light transition-colors"
            >
              {event.session_id.slice(0, 8)}...
            </Link>
          </>
        )}
      </div>
      {event.summary && (
        <p className="text-sm text-muted/80 whitespace-pre-wrap break-words">
          {event.summary}
        </p>
      )}
    </div>
  )
}

// ===== Main component =====

export default function TraceDetail() {
  const { traceId } = useParams<{ traceId: string }>()
  const { data: events, isLoading: eventsLoading, error: eventsError, refetch: refetchEvents } = useTraceDetail(traceId ?? '')
  const { data: messages, isLoading: messagesLoading, error: messagesError, refetch: refetchMessages } = useTraceMessages(traceId ?? '')
  const { data: llmCalls } = useTraceLlmCalls(traceId ?? '')
  const { data: toolCalls } = useTraceToolCalls(traceId ?? '')
  const [investigationScope, setInvestigationScope] = useState<InvestigationScope | null>(null)
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())

  const openInvestigation = (
    messageId: number,
    toolCallIndex?: number,
    toolName?: string,
  ) => {
    // Find the message to get session_id and agent_id
    const msg = messages?.find((m) => m.id === messageId)
    setInvestigationScope({
      type: toolCallIndex != null ? 'tool_call' : 'message',
      messageId,
      toolCallIndex,
      toolName,
      sessionId: msg?.session_id ?? '',
      agentId: msg?.agent_id ?? '',
    })
  }

  const isLoading = eventsLoading || messagesLoading
  const error = eventsError || messagesError

  // Non-message events (audit, task)
  const nonMessageEvents = events?.filter((e) => e.event_type !== 'message') ?? []

  // Count summary
  const messageBadgeCount = messages?.length ?? 0
  const auditCount = events?.filter((e) => e.event_type === 'audit').length ?? 0
  const taskCount = events?.filter((e) => e.event_type === 'task').length ?? 0
  const totalEvents = messageBadgeCount + auditCount + taskCount

  // Build a unified timeline: messages + non-message events, sorted by created_at
  type TimelineItem =
    | { kind: 'message'; msg: TraceMessage }
    | { kind: 'event'; event: typeof nonMessageEvents[number] }

  const timeline: TimelineItem[] = []
  if (messages) {
    for (const msg of messages) {
      timeline.push({ kind: 'message', msg })
    }
  }
  for (const event of nonMessageEvents) {
    timeline.push({ kind: 'event', event })
  }
  timeline.sort((a, b) => {
    const tsA = a.kind === 'message' ? a.msg.created_at : a.event.created_at
    const tsB = b.kind === 'message' ? b.msg.created_at : b.event.created_at
    return new Date(tsA).getTime() - new Date(tsB).getTime()
  })

  return (
    <div>
      <div className="flex items-center gap-3 mb-6">
        <Link
          to="/"
          className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
        >
          <ArrowLeft size={18} />
        </Link>
        <div>
          <h2 className="text-heading text-xl font-semibold">Trace Detail</h2>
          <p className="text-xs text-accent font-mono mt-0.5">{traceId}</p>
        </div>
      </div>

      {/* Event count summary */}
      {totalEvents > 0 && (
        <div className="flex items-center gap-3 mb-5">
          <span className="text-xs text-muted/60">{totalEvents} events in this trace</span>
          <span className="text-white/[0.06]">|</span>
          {[
            { type: 'message', count: messageBadgeCount },
            { type: 'audit', count: auditCount },
            { type: 'task', count: taskCount },
          ].map(({ type: t, count }) => {
            if (count === 0) return null
            const badge = eventTypeBadge(t)
            return (
              <span
                key={t}
                className={`inline-flex items-center gap-1.5 text-[10px] font-medium px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
              >
                {count} {t}
              </span>
            )
          })}
        </div>
      )}

      {isLoading ? (
        <LoadingState variant="detail" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => { refetchEvents(); refetchMessages() }} />
      ) : timeline.length === 0 ? (
        <EmptyState message="No events found for this trace" />
      ) : (
        <div className="space-y-3">
          {timeline.map((item, i) =>
            item.kind === 'message' ? (
              <MessageCard key={`msg-${item.msg.id}`} msg={item.msg} onInvestigate={openInvestigation} />
            ) : (
              <EventCard key={`evt-${i}`} event={item.event} />
            ),
          )}
        </div>
      )}

      {/* LLM Calls section */}
      {llmCalls && llmCalls.length > 0 && (
        <div className="mt-8">
          <div className="flex items-center gap-2 mb-3">
            <Brain size={16} className="text-muted/40" />
            <h3 className="text-heading text-sm font-semibold">
              LLM Calls
            </h3>
            <span className="text-[10px] text-muted/40">
              {llmCalls.length} call{llmCalls.length !== 1 ? 's' : ''}
            </span>
          </div>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                  <th className="text-left px-4 py-3 font-medium">Provider</th>
                  <th className="text-left px-4 py-3 font-medium">Model</th>
                  <th className="text-right px-4 py-3 font-medium">Input</th>
                  <th className="text-right px-4 py-3 font-medium">Output</th>
                  <th className="text-right px-4 py-3 font-medium">Latency</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {llmCalls.map((row) => (
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
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right whitespace-nowrap">{formatLatency(row.latency_ms)}</td>
                    <td className="px-4 py-3"><StatusBadge {...llmStatusVariant(row.status)} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Tool Calls section */}
      {toolCalls && toolCalls.length > 0 && (
        <div className="mt-8">
          <div className="flex items-center gap-2 mb-3">
            <Wrench size={16} className="text-muted/40" />
            <h3 className="text-heading text-sm font-semibold">
              Tool Calls
            </h3>
            <span className="text-[10px] text-muted/40">
              {toolCalls.length} call{toolCalls.length !== 1 ? 's' : ''}
            </span>
          </div>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="w-8 px-2 py-3" />
                  <th className="text-left px-4 py-3 font-medium">Tool</th>
                  <th className="text-left px-4 py-3 font-medium">Source</th>
                  <th className="text-left px-4 py-3 font-medium">Skill</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-right px-4 py-3 font-medium">Latency</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {toolCalls.map((row) => {
                  const isOpen = expandedToolCalls.has(row.id)
                  return (
                    <Fragment key={row.id}>
                      <tr
                        onClick={() => {
                          setExpandedToolCalls((prev) => {
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
                        <td className="px-4 py-3 text-xs font-mono font-medium">
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
                      </tr>
                      {isOpen && (
                        <tr>
                          <td colSpan={6} className="px-6 py-4 bg-white/[0.02]">
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
        </div>
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
