import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useAgentDetail, useAgentSessions, useAgentAudit } from '../api/agents.ts'
import StatusBadge from '../components/StatusBadge.tsx'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatRelativeTime } from '../hooks/useFormatTime.ts'
import { ArrowLeft, Eye, Code } from 'lucide-react'

type MemoryView = 'raw' | 'view'

export default function AgentDetail() {
  const { agentId } = useParams<{ agentId: string }>()
  const [sessionsPage, setSessionsPage] = useState(1)
  const [auditPage] = useState(1)
  const [memoryView, setMemoryView] = useState<MemoryView>('view')

  const { data: agent, isLoading, error } = useAgentDetail(agentId ?? '')
  const { data: sessions } = useAgentSessions(agentId ?? '', sessionsPage)
  const { data: audit } = useAgentAudit(agentId ?? '', auditPage)

  if (isLoading) {
    return <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
  }
  if (error || !agent) {
    return (
      <div className="text-red-400 py-8 text-center text-sm">
        {error instanceof Error ? error.message : 'Agent not found'}
      </div>
    )
  }

  // Calculate total core memory usage
  const totalTokens = agent.core_memory.reduce((sum, m) => sum + m.token_count, 0)
  const maxTokens = 2000
  const usagePercent = Math.min(100, Math.round((totalTokens / maxTokens) * 100))

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Link
            to="/agents"
            className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
          >
            <ArrowLeft size={18} />
          </Link>
          <div className="w-10 h-10 rounded-xl bg-accent/10 flex items-center justify-center">
            <span className="text-accent font-semibold">
              {agent.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div>
            <div className="flex items-center gap-3">
              <h2 className="text-heading text-xl font-semibold">{agent.name}</h2>
              <StatusBadge active={agent.active} />
            </div>
            <p className="text-xs text-muted/50 font-mono mt-0.5">
              ID: {agent.id} &middot; Created {formatRelativeTime(agent.created_at)}
            </p>
          </div>
        </div>
      </div>

      {/* Two-column layout: Core Memory + Soul.md */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 mb-6">
        {/* Core Memory */}
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-heading text-sm font-semibold">Core Memory</h3>
            <div className="flex items-center gap-1 bg-white/[0.04] rounded-lg p-0.5">
              <button
                onClick={() => setMemoryView('view')}
                className={`p-1 rounded text-xs ${memoryView === 'view' ? 'bg-accent/20 text-accent' : 'text-muted/50 hover:text-muted'}`}
              >
                <Eye size={14} />
              </button>
              <button
                onClick={() => setMemoryView('raw')}
                className={`p-1 rounded text-xs ${memoryView === 'raw' ? 'bg-accent/20 text-accent' : 'text-muted/50 hover:text-muted'}`}
              >
                <Code size={14} />
              </button>
            </div>
          </div>
          {agent.core_memory.length === 0 ? (
            <EmptyState message="No core memory blocks" />
          ) : memoryView === 'raw' ? (
            <pre className="bg-bg rounded-xl p-4 text-xs font-mono text-muted/80 overflow-auto max-h-80 border border-white/[0.04]">
              {agent.core_memory.map((m) => `${m.key}: ${JSON.stringify(m.value)}`).join('\n')}
            </pre>
          ) : (
            <div className="space-y-2">
              {agent.core_memory.map((mem) => (
                <div
                  key={mem.key}
                  className="flex items-start gap-3 py-2 border-b border-white/[0.03] last:border-0"
                >
                  <span className="text-xs text-accent font-mono font-medium min-w-[120px] shrink-0 pt-0.5">
                    {mem.key}:
                  </span>
                  <span className="text-xs text-muted/80 font-mono break-words">
                    {mem.value.length > 100 ? `"${mem.value.slice(0, 100)}..."` : `"${mem.value}"`}
                  </span>
                </div>
              ))}
            </div>
          )}
          {/* Memory usage bar */}
          <div className="mt-4 pt-3 border-t border-white/[0.04]">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] text-muted/50 uppercase tracking-wider">Memory Usage</span>
              <span className="text-[10px] text-muted/60 font-mono">{usagePercent}%</span>
            </div>
            <div className="h-1.5 bg-white/[0.04] rounded-full overflow-hidden">
              <div
                className="h-full bg-accent rounded-full transition-all"
                style={{ width: `${usagePercent}%` }}
              />
            </div>
          </div>
        </div>

        {/* Soul.md */}
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
          <h3 className="text-heading text-sm font-semibold mb-4">soul.md</h3>
          {agent.soul_md ? (
            <div className="bg-bg rounded-xl p-4 border border-white/[0.04] max-h-96 overflow-y-auto">
              <pre className="text-xs text-muted/80 whitespace-pre-wrap font-mono leading-relaxed">
                {agent.soul_md}
              </pre>
            </div>
          ) : (
            <EmptyState message="No soul.md defined" />
          )}
        </div>
      </div>

      {/* Recent Audit Events */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mb-4">
        <h3 className="text-heading text-sm font-semibold mb-4">Recent Audit Events</h3>
        {!audit || audit.data.length === 0 ? (
          <EmptyState message="No audit events for this agent" />
        ) : (
          <div className="space-y-3">
            {audit.data.slice(0, 5).map((e) => (
              <div
                key={e.id}
                className="flex items-start gap-3 py-2 border-b border-white/[0.03] last:border-0"
              >
                <span className="text-[10px] text-muted/50 font-mono whitespace-nowrap pt-0.5">
                  {e.created_at}
                </span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-amber-400 font-mono font-medium uppercase">
                      {e.tool_name}
                    </span>
                    <span className="text-xs text-muted/40">{e.target_key}</span>
                  </div>
                  {e.after_value && (
                    <p className="text-xs text-muted/60 mt-0.5 truncate">
                      {e.before_value && (
                        <span className="text-red-400/60 line-through mr-1">{e.before_value.slice(0, 40)}</span>
                      )}
                      <span className="text-emerald-400/60">{e.after_value.slice(0, 60)}</span>
                    </p>
                  )}
                </div>
                <span className="text-[10px] text-muted/30 font-mono shrink-0">
                  {e.session_id?.slice(0, 8)}
                </span>
              </div>
            ))}
            {audit.total > 5 && (
              <div className="text-center pt-2">
                <span className="text-xs text-accent hover:text-accent-light cursor-pointer">
                  View Full Logs ({audit.total} total)
                </span>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Recent Sessions */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-heading text-sm font-semibold">Recent Sessions</h3>
          {sessions && sessions.total > 0 && (
            <span className="text-xs text-muted/50">{sessions.total} total</span>
          )}
        </div>
        {!sessions || sessions.data.length === 0 ? (
          <EmptyState message="No sessions for this agent" />
        ) : (
          <>
            <div className="overflow-hidden rounded-xl border border-white/[0.04]">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-white/[0.04] text-muted/50 text-xs uppercase tracking-wider bg-white/[0.02]">
                    <th className="text-left px-4 py-2.5 font-medium">Session ID</th>
                    <th className="text-left px-4 py-2.5 font-medium">Duration</th>
                    <th className="text-left px-4 py-2.5 font-medium">Messages</th>
                    <th className="text-left px-4 py-2.5 font-medium">Channel</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/[0.03]">
                  {sessions.data.map((s) => (
                    <tr key={s.id} className="hover:bg-white/[0.02] transition-colors">
                      <td className="px-4 py-2.5">
                        <Link
                          to={`/sessions/${s.id}`}
                          className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                        >
                          {s.id}
                        </Link>
                      </td>
                      <td className="px-4 py-2.5 text-xs text-muted font-mono">
                        {s.ended_at
                          ? `${Math.round((s.ended_at - s.started_at) / 60)}m`
                          : 'ongoing'}
                      </td>
                      <td className="px-4 py-2.5 text-xs text-heading font-medium">
                        {s.message_count}
                      </td>
                      <td className="px-4 py-2.5">
                        <span className="text-xs text-muted bg-white/[0.04] px-1.5 py-0.5 rounded">
                          {s.channel_type}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <Pagination
              page={sessionsPage}
              perPage={50}
              total={sessions.total}
              onPageChange={setSessionsPage}
            />
          </>
        )}
      </div>
    </div>
  )
}
