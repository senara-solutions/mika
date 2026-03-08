import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useSessionDetail, useSessionMessages } from '../api/sessions.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import { ArrowLeft } from 'lucide-react'

function roleStyle(role: string) {
  switch (role) {
    case 'user':
      return {
        bg: 'bg-accent/10 border-accent/20',
        label: 'text-accent-light',
        name: 'User',
      }
    case 'assistant':
      return {
        bg: 'bg-bg-card border-white/[0.05]',
        label: 'text-heading',
        name: 'Assistant',
      }
    case 'system':
    case 'summary':
      return {
        bg: 'bg-white/[0.02] border-white/[0.04]',
        label: 'text-muted/60',
        name: role.charAt(0).toUpperCase() + role.slice(1),
      }
    case 'tool_result':
      return {
        bg: 'bg-emerald-400/5 border-emerald-400/20',
        label: 'text-emerald-400',
        name: 'Tool Result',
      }
    default:
      return {
        bg: 'bg-bg-card border-white/[0.05]',
        label: 'text-muted',
        name: role,
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
      <div className="flex items-center gap-3 mb-6">
        <Link
          to="/sessions"
          className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
        >
          <ArrowLeft size={18} />
        </Link>
        <div>
          <h2 className="text-heading text-xl font-semibold">Session Detail</h2>
          {session && (
            <div className="flex items-center gap-3 mt-0.5">
              <span className="text-xs text-muted font-mono">{session.id}</span>
              <span className="text-xs text-muted/60">&middot;</span>
              <span className="text-xs text-heading">{session.agent_id}</span>
              <span className="text-xs text-muted/60">&middot;</span>
              <span className="text-xs text-muted bg-white/[0.04] px-1.5 py-0.5 rounded">
                {session.channel_type}
              </span>
              <span className="text-xs text-muted/60">&middot;</span>
              <span className="text-xs text-muted/80">
                {formatTimestamp(session.started_at)}
              </span>
            </div>
          )}
        </div>
      </div>

      {/* Messages */}
      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : !messages || messages.data.length === 0 ? (
        <EmptyState message="No messages in this session" />
      ) : (
        <>
          <div className="space-y-3">
            {messages.data.map((msg) => {
              const style = roleStyle(msg.role)
              return (
                <div key={msg.id} className={`border rounded-xl p-4 ${style.bg}`}>
                  <div className="flex items-center gap-2 mb-2">
                    <span className={`text-xs font-semibold ${style.label}`}>
                      {style.name}
                    </span>
                    <span className="text-xs text-muted/40 ml-auto">
                      {formatTimestamp(msg.created_at)}
                    </span>
                  </div>
                  <div className="text-sm text-muted/80 whitespace-pre-wrap break-words max-h-96 overflow-y-auto">
                    {msg.content}
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
