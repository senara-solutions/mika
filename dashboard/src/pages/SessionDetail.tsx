import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useSessionDetail, useSessionMessages } from '../api/sessions.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import { ArrowLeft, User, Bot, Settings, Wrench, Download } from 'lucide-react'

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
              <h2 className="text-heading text-lg font-semibold font-mono">
                sess_{sessionId?.slice(0, 10)}
              </h2>
              {session && (
                <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-emerald-500/15 text-emerald-400">
                  Active
                </span>
              )}
            </div>
            {session && (
              <div className="flex items-center gap-3 mt-1">
                <span className="text-xs text-muted/60">
                  Session initialized via {session.channel_type} &middot; {formatTimestamp(session.started_at)}
                </span>
              </div>
            )}
          </div>
        </div>
        <button className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-white/[0.04] border border-white/[0.06] text-xs text-muted hover:text-heading hover:border-white/[0.1] transition-colors">
          <Download size={13} />
          Export
        </button>
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
