import { useState, Fragment } from 'react'
import { useParams, Link } from 'react-router'
import { useSessionDetail, useSessionMessages } from '../api/sessions.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import {
  ArrowLeft,
  User,
  Bot,
  Settings,
  Wrench,
  Copy,
  Check,
  ChevronRight,
  ChevronDown,
} from 'lucide-react'

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

const QUICK_COPY_KEYS: Record<string, string> = {
  run_shell: 'command',
  read_workspace: 'path',
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

function CopyButton({
  text,
  className,
  title = 'Copy to clipboard',
}: {
  text: string
  className?: string
  title?: string
}) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // Silently fail — clipboard may be unavailable
    }
  }

  return (
    <button
      onClick={handleCopy}
      className={`opacity-40 hover:opacity-100 transition-opacity shrink-0 ${className ?? ''}`}
      title={title}
    >
      {copied ? <Check size={12} className="text-emerald-400" /> : <Copy size={12} />}
    </button>
  )
}

function truncateText(text: string, maxLen = 80): string {
  if (text.length <= maxLen) return text
  // Strip backend's trailing "..." before re-truncating to avoid "text......"
  const cleaned = text.endsWith('...') ? text.slice(0, -3) : text
  if (cleaned.length <= maxLen) return text
  return cleaned.slice(0, maxLen) + '...'
}

function ToolCallsTable({ metadata }: { metadata: string | null }) {
  const toolCalls = parseToolCalls(metadata)
  const [expanded, setExpanded] = useState<Set<number>>(new Set())

  if (toolCalls.length === 0) return null

  const toggleExpand = (index: number) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      next.has(index) ? next.delete(index) : next.add(index)
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
                  </tr>
                  {/* Expanded detail row */}
                  {isOpen && (
                    <tr>
                      <td colSpan={5} className="px-4 py-3 bg-white/[0.02]">
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

export default function SessionDetail() {
  const { sessionId } = useParams<{ sessionId: string }>()
  const [page, setPage] = useState(1)

  const { data: session, isLoading: sessionLoading } = useSessionDetail(sessionId ?? '')
  const { data: messages, isLoading: messagesLoading } = useSessionMessages(
    sessionId ?? '',
    page,
  )

  const isLoading = sessionLoading || messagesLoading

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
            </div>
            {session && (
              <div className="flex items-center gap-3 mt-1">
                <span className="text-xs text-muted/60">
                  Session initialized via {session.channel_type} &middot;{' '}
                  {formatTimestamp(session.started_at)}
                </span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Session info bar */}
      {session && (
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
        </div>
      )}

      {/* Messages */}
      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : !messages || messages.data.length === 0 ? (
        <EmptyState message="No messages in this session" />
      ) : (
        <>
          <div className="space-y-3">
            {messages.data.map((msg) => {
              const config = roleConfig(msg.role)
              return (
                <div key={msg.id} className={config.align}>
                  <div className={`border rounded-xl p-4 ${config.bg}`}>
                    <div className="flex items-center gap-2 mb-2">
                      <div
                        className={`w-6 h-6 rounded-md flex items-center justify-center ${config.iconBg}`}
                      >
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
                    {msg.role === 'assistant' && <ToolCallsTable metadata={msg.metadata} />}
                  </div>
                </div>
              )
            })}
          </div>
          <Pagination
            page={page}
            perPage={50}
            total={messages.total}
            onPageChange={setPage}
          />
        </>
      )}
    </div>
  )
}
