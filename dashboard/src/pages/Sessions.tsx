import { useState } from 'react'
import { Link } from 'react-router'
import { useSessions, type SessionsFilters } from '../api/sessions.ts'
import { useAgents } from '../api/agents.ts'
import { Pagination, EmptyState, LoadingState, ErrorState, formatApiError, ListRow, AgentFilter, SelectFilter, TimeRangeFilter, LiveRefreshToggle, formatRelativeTime } from '@senara-solutions/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { useLiveRefresh } from '../hooks/useLiveRefresh.ts'
import { Search, Terminal, MessageSquare, Users, Settings, ArrowRightLeft } from 'lucide-react'

const CHANNEL_OPTIONS = [
  { label: 'All Channels', value: '' },
  { label: 'CLI', value: 'cli' },
  { label: 'Telegram', value: 'telegram' },
  { label: 'Team', value: 'team' },
  { label: 'System', value: 'system' },
  { label: 'Delegate', value: 'delegate' },
]

function channelIcon(type: string) {
  switch (type) {
    case 'cli':
      return <Terminal size={12} />
    case 'telegram':
      return <MessageSquare size={12} />
    case 'team':
      return <Users size={12} />
    case 'system':
      return <Settings size={12} />
    case 'delegate':
      return <ArrowRightLeft size={12} />
    default:
      return <MessageSquare size={12} />
  }
}

export default function Sessions() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()

  const filters: SessionsFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    channel_type: searchParams.get('channel_type') ?? undefined,
    session_id: searchParams.get('session_id') ?? undefined,
    from: searchParams.get('from') ?? undefined,
    to: searchParams.get('to') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const isDefaultView =
    !filters.agent_id &&
    !filters.channel_type &&
    !filters.session_id &&
    !filters.from &&
    !filters.to &&
    (filters.page ?? 1) === 1

  const { isLive, isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
    defaultEnabled: false,
    interval: 15_000,
    isDefaultView,
  })

  const [sessionSearch, setSessionSearch] = useState(filters.session_id ?? '')

  const { data, isLoading, error, refetch } = useSessions(filters, refetchInterval)
  const { data: agents } = useAgents()

  return (
    <div>
      <div className="flex items-center justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Sessions</h2>
          <p className="text-sm text-muted/60 mt-1">
            {data ? `${data.total} session${data.total !== 1 ? 's' : ''} found` : 'Loading sessions...'}
          </p>
        </div>
        <LiveRefreshToggle
          isLive={isEffectivelyLive}
          onToggle={toggle}
          disabled={!isDefaultView && isLive}
        />
      </div>

      {/* Filters */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <AgentFilter
            agents={agents}
            value={filters.agent_id ?? ''}
            onChange={(v) => updateFilter('agent_id', v)}
          />
          <div className="relative flex-1 min-w-[180px]">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted/40" />
            <input
              type="text"
              aria-label="Search session ID"
              placeholder="Search session ID..."
              value={sessionSearch}
              onChange={(e) => setSessionSearch(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && updateFilter('session_id', sessionSearch.trim())}
              className="w-full bg-bg border border-white/[0.06] rounded-lg pl-9 pr-3 py-2 text-sm font-mono text-muted placeholder:text-muted/30 placeholder:font-sans focus:outline-none focus:border-accent/40"
            />
          </div>
          <SelectFilter
            ariaLabel="Filter by channel"
            value={filters.channel_type ?? ''}
            onChange={(v) => updateFilter('channel_type', v)}
            options={CHANNEL_OPTIONS}
          />
          <TimeRangeFilter
            value={{ from: filters.from, to: filters.to }}
            onChange={(range) => {
              updateFilter('from', range.from ?? '')
              updateFilter('to', range.to ?? '')
            }}
          />
          {(filters.agent_id || filters.channel_type || filters.session_id || filters.from || filters.to) && (
            <button
              onClick={() => setSearchParams(new URLSearchParams())}
              className="text-xs text-muted/60 hover:text-muted transition-colors"
            >
              Clear
            </button>
          )}
        </div>
      </div>

      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState
          message="No sessions match your filters"
          action={(filters.agent_id || filters.channel_type || filters.session_id || filters.from || filters.to)
            ? { label: 'Clear filters', onClick: () => setSearchParams(new URLSearchParams()) }
            : undefined}
        />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Session</th>
                  <th className="text-left px-4 py-3 font-medium">Agent</th>
                  <th className="text-left px-4 py-3 font-medium">Channel</th>
                  <th className="text-left px-4 py-3 font-medium">Started</th>
                  <th className="text-left px-4 py-3 font-medium">Messages</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((s) => (
                  <ListRow key={s.id} variant="static">
                    <td className="px-4 py-3">
                      <Link
                        to={`/sessions/${s.id}`}
                        className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                      >
                        {s.id}
                      </Link>
                    </td>
                    <td className="px-4 py-3 text-xs text-heading font-medium">{s.agent_id}</td>
                    <td className="px-4 py-3">
                      <span className="inline-flex items-center gap-1.5 text-xs text-muted bg-white/[0.04] px-2 py-0.5 rounded-full">
                        {channelIcon(s.channel_type)}
                        {s.channel_type}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                      {formatRelativeTime(s.started_at)}
                    </td>
                    <td className="px-4 py-3 text-xs text-heading font-medium">{s.message_count}</td>
                  </ListRow>
                ))}
              </tbody>
            </table>
          </div>
          <Pagination
            page={filters.page ?? 1}
            perPage={filters.per_page ?? 50}
            total={data.total}
            onPageChange={setPage}
          />
        </>
      )}
    </div>
  )
}
