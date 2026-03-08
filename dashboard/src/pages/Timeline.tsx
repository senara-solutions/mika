import { useState } from 'react'
import { Link, useSearchParams } from 'react-router'
import { useTimeline, type TimelineFilters } from '../api/timeline.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'

const EVENT_TYPES = ['', 'message', 'audit', 'task']

function eventTypeColor(type: string): string {
  switch (type) {
    case 'message':
      return 'text-blue-400'
    case 'audit':
      return 'text-amber-400'
    case 'task':
      return 'text-emerald-400'
    default:
      return 'text-muted'
  }
}

export default function Timeline() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [traceSearch, setTraceSearch] = useState(searchParams.get('trace_id') ?? '')

  const filters: TimelineFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    event_type: searchParams.get('event_type') ?? undefined,
    trace_id: searchParams.get('trace_id') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useTimeline(filters)

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

  function handleTraceSearch() {
    updateFilter('trace_id', traceSearch.trim())
  }

  return (
    <div>
      <h2 className="text-heading text-xl font-semibold mb-4">Unified Timeline</h2>

      {/* Filter bar */}
      <div className="flex flex-wrap gap-3 mb-4">
        <select
          value={filters.event_type ?? ''}
          onChange={(e) => updateFilter('event_type', e.target.value)}
          className="bg-bg-card border border-white/[0.06] rounded-lg px-3 py-1.5 text-sm text-muted focus:outline-none focus:border-accent/40"
        >
          {EVENT_TYPES.map((t) => (
            <option key={t} value={t}>
              {t || 'All types'}
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

        <div className="flex gap-1">
          <input
            type="text"
            placeholder="Search trace_id..."
            value={traceSearch}
            onChange={(e) => setTraceSearch(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleTraceSearch()}
            className="bg-bg-card border border-white/[0.06] rounded-lg px-3 py-1.5 text-sm text-muted placeholder:text-muted/40 focus:outline-none focus:border-accent/40 w-64 font-mono"
          />
          <button
            onClick={handleTraceSearch}
            className="bg-bg-card border border-white/[0.06] rounded-lg px-3 py-1.5 text-sm text-muted hover:border-accent/40 transition-colors"
          >
            Go
          </button>
        </div>
      </div>

      {/* Table */}
      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No events match your filters" />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Time</th>
                  <th className="text-left px-4 py-3 font-medium">Agent</th>
                  <th className="text-left px-4 py-3 font-medium">Type</th>
                  <th className="text-left px-4 py-3 font-medium">Subtype</th>
                  <th className="text-left px-4 py-3 font-medium">Summary</th>
                  <th className="text-left px-4 py-3 font-medium">Trace</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((row, i) => (
                  <tr key={i} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-4 py-2.5 text-muted/80 whitespace-nowrap font-mono text-xs">
                      {formatTimestamp(row.created_at)}
                    </td>
                    <td className="px-4 py-2.5 text-heading text-xs">
                      {row.agent_id}
                    </td>
                    <td className="px-4 py-2.5">
                      <span
                        className={`text-xs font-medium px-1.5 py-0.5 rounded ${eventTypeColor(row.event_type)}`}
                      >
                        {row.event_type}
                      </span>
                    </td>
                    <td className="px-4 py-2.5 text-muted text-xs font-mono">
                      {row.event_subtype}
                    </td>
                    <td className="px-4 py-2.5 text-muted/80 text-xs max-w-md truncate">
                      {row.summary}
                    </td>
                    <td className="px-4 py-2.5">
                      {row.trace_id ? (
                        <Link
                          to={`/traces/${row.trace_id}`}
                          className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                        >
                          {row.trace_id.slice(0, 12)}...
                        </Link>
                      ) : (
                        <span className="text-muted/30 text-xs">-</span>
                      )}
                    </td>
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
