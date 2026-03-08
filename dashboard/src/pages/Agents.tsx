import { Link } from 'react-router'
import { useAgents } from '../api/agents.ts'
import StatusBadge from '../components/StatusBadge.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatRelativeTime } from '../hooks/useFormatTime.ts'
import { MessageSquare } from 'lucide-react'

export default function Agents() {
  const { data: agents, isLoading, error } = useAgents()

  return (
    <div>
      <h2 className="text-heading text-xl font-semibold mb-4">Agents</h2>

      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !agents || agents.length === 0 ? (
        <EmptyState message="No agents found" />
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {agents.map((agent) => (
            <Link
              key={agent.id}
              to={`/agents/${agent.id}`}
              className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 hover:border-accent/40 hover:shadow-[0_0_30px_rgba(124,106,247,0.08)] transition-all"
            >
              <div className="flex items-start justify-between mb-3">
                <div>
                  <h3 className="text-heading font-semibold">{agent.name}</h3>
                  <p className="text-xs text-muted/60 font-mono mt-0.5">{agent.id}</p>
                </div>
                <StatusBadge active={agent.active} />
              </div>
              <div className="flex items-center gap-4 text-xs text-muted">
                <span className="flex items-center gap-1.5">
                  <MessageSquare size={12} />
                  {agent.message_count} messages
                </span>
                {agent.last_seen && (
                  <span>Last seen {formatRelativeTime(agent.last_seen)}</span>
                )}
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  )
}
