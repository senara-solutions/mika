import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useAgentDetail, useAgentSessions, useAgentAudit } from '../api/agents.ts'
import StatusBadge from '../components/StatusBadge.tsx'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp, formatRelativeTime } from '../hooks/useFormatTime.ts'
import { ArrowLeft } from 'lucide-react'

type Tab = 'overview' | 'sessions' | 'audit'

export default function AgentDetail() {
  const { agentId } = useParams<{ agentId: string }>()
  const [tab, setTab] = useState<Tab>('overview')
  const [sessionsPage, setSessionsPage] = useState(1)
  const [auditPage, setAuditPage] = useState(1)

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

  const tabs: { key: Tab; label: string }[] = [
    { key: 'overview', label: 'Overview' },
    { key: 'sessions', label: 'Sessions' },
    { key: 'audit', label: 'Audit Log' },
  ]

  return (
    <div>
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <Link
          to="/agents"
          className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
        >
          <ArrowLeft size={18} />
        </Link>
        <div className="flex-1">
          <div className="flex items-center gap-3">
            <h2 className="text-heading text-xl font-semibold">{agent.name}</h2>
            <StatusBadge active={agent.active} />
          </div>
          <p className="text-xs text-muted/60 mt-0.5">
            {agent.message_count} messages
            {agent.last_seen && <> &middot; Last seen {formatRelativeTime(agent.last_seen)}</>}
          </p>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6 border-b border-white/[0.05]">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`px-4 py-2 text-sm border-b-2 transition-colors -mb-px ${
              tab === t.key
                ? 'border-accent text-accent-light font-medium'
                : 'border-transparent text-muted hover:text-heading'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {tab === 'overview' && (
        <div className="space-y-6">
          {/* Core Memory */}
          <div>
            <h3 className="text-heading text-sm font-semibold mb-3">Core Memory</h3>
            {agent.core_memory.length === 0 ? (
              <EmptyState message="No core memory blocks" />
            ) : (
              <div className="grid gap-3">
                {agent.core_memory.map((mem) => (
                  <div
                    key={mem.key}
                    className="bg-bg-card border border-white/[0.05] rounded-xl p-4"
                  >
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-xs text-accent font-mono font-medium">
                        {mem.key}
                      </span>
                      <span className="text-xs text-muted/40">
                        {mem.token_count} tokens &middot; {mem.updated_at}
                      </span>
                    </div>
                    <p className="text-sm text-muted/80 whitespace-pre-wrap">{mem.value}</p>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Soul.md */}
          {agent.soul_md && (
            <div>
              <h3 className="text-heading text-sm font-semibold mb-3">soul.md</h3>
              <div className="bg-bg-card border border-white/[0.05] rounded-xl p-4">
                <pre className="text-sm text-muted/80 whitespace-pre-wrap font-mono">
                  {agent.soul_md}
                </pre>
              </div>
            </div>
          )}
        </div>
      )}

      {tab === 'sessions' && (
        <div>
          {!sessions || sessions.data.length === 0 ? (
            <EmptyState message="No sessions for this agent" />
          ) : (
            <>
              <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                      <th className="text-left px-4 py-3 font-medium">Session</th>
                      <th className="text-left px-4 py-3 font-medium">Channel</th>
                      <th className="text-left px-4 py-3 font-medium">Started</th>
                      <th className="text-left px-4 py-3 font-medium">Messages</th>
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
                            {s.id.slice(0, 12)}...
                          </Link>
                        </td>
                        <td className="px-4 py-2.5 text-xs text-muted">{s.channel_type}</td>
                        <td className="px-4 py-2.5 text-xs text-muted/80">
                          {formatTimestamp(s.started_at)}
                        </td>
                        <td className="px-4 py-2.5 text-xs text-muted">{s.message_count}</td>
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
      )}

      {tab === 'audit' && (
        <div>
          {!audit || audit.data.length === 0 ? (
            <EmptyState message="No audit events for this agent" />
          ) : (
            <>
              <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                      <th className="text-left px-4 py-3 font-medium">Time</th>
                      <th className="text-left px-4 py-3 font-medium">Tool</th>
                      <th className="text-left px-4 py-3 font-medium">Target</th>
                      <th className="text-left px-4 py-3 font-medium">Change</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-white/[0.03]">
                    {audit.data.map((e) => (
                      <tr key={e.id} className="hover:bg-white/[0.02] transition-colors">
                        <td className="px-4 py-2.5 text-xs text-muted/80 whitespace-nowrap">
                          {e.created_at}
                        </td>
                        <td className="px-4 py-2.5 text-xs text-accent font-mono">
                          {e.tool_name}
                        </td>
                        <td className="px-4 py-2.5 text-xs text-heading">{e.target_key}</td>
                        <td className="px-4 py-2.5 text-xs text-muted max-w-sm truncate">
                          {e.before_value ? `${e.before_value} → ` : ''}
                          {e.after_value}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <Pagination
                page={auditPage}
                perPage={50}
                total={audit.total}
                onPageChange={setAuditPage}
              />
            </>
          )}
        </div>
      )}
    </div>
  )
}
