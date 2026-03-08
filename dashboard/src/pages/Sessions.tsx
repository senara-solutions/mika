import { Link, useSearchParams } from 'react-router'
import { useSessions, type SessionsFilters } from '../api/sessions.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'

const CHANNEL_TYPES = ['', 'cli', 'telegram', 'team', 'system']

export default function Sessions() {
  const [searchParams, setSearchParams] = useSearchParams()

  const filters: SessionsFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    channel_type: searchParams.get('channel_type') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useSessions(filters)

  function updateFilter(key: string, value: string) {
    const next = new URLSearchParams(searchParams)
    if (value) {
      next.set(key, value)
    } else {
      next.delete(key)
    }
    next.delete('page')
    setSearchParams(next)
  }

  function setPage(page: number) {
    const next = new URLSearchParams(searchParams)
    next.set('page', String(page))
    setSearchParams(next)
  }

  return (
    <div>
      <h2 className="text-heading text-xl font-semibold mb-4">Sessions</h2>

      {/* Filters */}
      <div className="flex flex-wrap gap-3 mb-4">
        <select
          value={filters.channel_type ?? ''}
          onChange={(e) => updateFilter('channel_type', e.target.value)}
          className="bg-bg-card border border-white/[0.06] rounded-lg px-3 py-1.5 text-sm text-muted focus:outline-none focus:border-accent/40"
        >
          {CHANNEL_TYPES.map((t) => (
            <option key={t} value={t}>
              {t || 'All channels'}
            </option>
          ))}
        </select>

        <input
          type="text"
          placeholder="Filter by agent..."
          value={filters.agent_id ?? ''}
          onChange={(e) => updateFilter('agent_id', e.target.value)}
          className="bg-bg-card border border-white/[0.06] rounded-lg px-3 py-1.5 text-sm text-muted placeholder:text-muted/40 focus:outline-none focus:border-accent/40 w-40"
        />
      </div>

      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No sessions match your filters" />
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
                  <tr key={s.id} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-4 py-2.5">
                      <Link
                        to={`/sessions/${s.id}`}
                        className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                      >
                        {s.id.slice(0, 16)}...
                      </Link>
                    </td>
                    <td className="px-4 py-2.5 text-xs text-heading">{s.agent_id}</td>
                    <td className="px-4 py-2.5">
                      <span className="text-xs text-muted bg-white/[0.04] px-1.5 py-0.5 rounded">
                        {s.channel_type}
                      </span>
                    </td>
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
