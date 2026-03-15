import { useState } from 'react'
import { useParams, Link } from 'react-router'
import { useAgentDetail, useAgentSessions, useAgentAudit, type CoreMemory } from '../api/agents.ts'
import StatusBadge from '../components/StatusBadge.tsx'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatRelativeTime } from '../utils/formatTime.ts'
import { ArrowLeft, User, Brain, Target, Users, GitBranch, Pencil } from 'lucide-react'

const BLOCK_TOKEN_CAP = 500
const EDIT_BUDGET = 3

const BLOCK_ICONS: Record<string, typeof User> = {
  user_summary: User,
  self_model: Brain,
  current_priorities: Target,
  key_people: Users,
  workflows: GitBranch,
}

function TokenBar({ tokenCount }: { tokenCount: number }) {
  const pct = Math.min(100, Math.round((tokenCount / BLOCK_TOKEN_CAP) * 100))
  return (
    <div className="flex items-center gap-2 mt-2">
      <span className="text-[10px] text-muted/50 uppercase tracking-wider">Tokens</span>
      <div className="flex-1 h-1 bg-white/[0.06] rounded-full overflow-hidden">
        <div
          className="h-full bg-accent rounded-full transition-all"
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-[10px] text-muted/50 font-mono">{tokenCount}/{BLOCK_TOKEN_CAP}</span>
    </div>
  )
}

function MemoryBlock({ mem }: { mem: CoreMemory }) {
  const Icon = BLOCK_ICONS[mem.key] ?? Brain
  return (
    <div className="bg-bg rounded-lg p-3 border border-white/[0.04]">
      <div className="flex items-center gap-1.5 mb-1.5">
        <Icon size={12} className="text-accent shrink-0" />
        <span className="text-[11px] text-heading font-semibold uppercase tracking-wider truncate">
          {mem.key}
        </span>
      </div>
      <p className="text-xs text-muted/70 font-mono leading-relaxed line-clamp-3 min-h-[3lh]">
        {mem.value || '\u00A0'}
      </p>
      <TokenBar tokenCount={mem.token_count} />
    </div>
  )
}

export default function AgentDetail() {
  const { agentId } = useParams<{ agentId: string }>()
  const [sessionsPage, setSessionsPage] = useState(1)
  const [auditPage] = useState(1)

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

  const editsUsed = agent.core_memory_edits_this_session

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
            <div className="flex items-center gap-1.5 text-muted/50">
              <Pencil size={12} />
              <span className="text-[11px] font-mono">
                Edits: {editsUsed} / {EDIT_BUDGET} used this session
              </span>
            </div>
          </div>
          {agent.core_memory.length === 0 ? (
            <EmptyState message="No core memory blocks" />
          ) : (
            <>
              <div className="grid grid-cols-2 gap-2">
                {agent.core_memory
                  .filter((m) => m.key !== 'workflows')
                  .map((mem) => (
                    <MemoryBlock key={mem.key} mem={mem} />
                  ))}
              </div>
              {agent.core_memory
                .filter((m) => m.key === 'workflows')
                .map((mem) => (
                  <div key={mem.key} className="mt-2">
                    <MemoryBlock mem={mem} />
                  </div>
                ))}
            </>
          )}
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
