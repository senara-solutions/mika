import { useState } from 'react'
import { Link } from 'react-router'
import { useAgents } from '../api/agents.ts'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { StatusBadge, EmptyState, LoadingState, ErrorState, formatApiError, formatRelativeTime } from '@senara-solutions/ui'
import { MessageSquare, Search } from 'lucide-react'

export default function Agents() {
  const { data: agents, isLoading, error, refetch } = useAgents()
  const { searchParams, updateFilter } = useSearchParamsFilter()
  const committedSearch = searchParams.get('search') ?? ''
  const [search, setSearch] = useState(committedSearch)

  const filtered = agents?.filter(
    (a) =>
      !committedSearch || a.name.toLowerCase().includes(committedSearch.toLowerCase()) || a.id.includes(committedSearch),
  )

  return (
    <div>
      <div className="flex items-center justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Agents Overview</h2>
          <p className="text-sm text-muted/60 mt-1">
            {agents ? `${agents.length} agent${agents.length !== 1 ? 's' : ''} registered` : 'Loading agents...'}
          </p>
        </div>
        <div className="relative">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted/40" />
          <input
            type="text"
            placeholder="Search agents..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && updateFilter('search', search)}
            className="bg-bg-card border border-white/[0.06] rounded-lg pl-9 pr-3 py-2 text-sm text-muted placeholder:text-muted/30 focus:outline-none focus:border-accent/40 w-56"
          />
        </div>
      </div>

      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !filtered || filtered.length === 0 ? (
        <EmptyState message="No agents found" action={committedSearch ? { label: 'Clear search', onClick: () => { setSearch(''); updateFilter('search', '') } } : undefined} />
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {filtered.map((agent) => (
            <Link
              key={agent.id}
              to={`/agents/${agent.id}`}
              className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 hover:border-accent/30 hover:shadow-[0_0_30px_rgba(124,106,247,0.06)] transition-all group"
            >
              <div className="flex items-start justify-between mb-4">
                <div className="flex items-center gap-3">
                  <div className="w-10 h-10 rounded-xl bg-accent/10 flex items-center justify-center">
                    <span className="text-accent font-semibold text-sm">
                      {agent.name.charAt(0).toUpperCase()}
                    </span>
                  </div>
                  <div>
                    <h3 className="text-heading font-semibold group-hover:text-accent-light transition-colors">
                      {agent.name}
                    </h3>
                    <p className="text-[10px] text-muted/40 font-mono mt-0.5">
                      {agent.id}
                    </p>
                  </div>
                </div>
                <StatusBadge variant={agent.active ? 'success' : 'neutral'} label={agent.active ? 'Active' : 'Inactive'} />
              </div>
              <div className="flex items-center justify-between text-xs text-muted pt-3 border-t border-white/[0.04]">
                <div className="flex items-center gap-4">
                  <span className="flex items-center gap-1.5">
                    <MessageSquare size={12} className="text-muted/50" />
                    <span className="text-heading font-medium">{agent.message_count.toLocaleString()}</span>
                    <span className="text-muted/50">messages</span>
                  </span>
                </div>
                {agent.last_seen && (
                  <span className="text-muted/50">
                    Last seen {formatRelativeTime(agent.last_seen)}
                  </span>
                )}
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  )
}
