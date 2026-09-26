import { useState } from 'react'
import { Link } from 'react-router'
import { useTimeline, type TimelineFilters } from '../api/timeline.ts'
import { useAgents } from '../api/agents.ts'
import { Pagination, EmptyState, LoadingState, ErrorState, formatApiError, ListRow, AgentFilter, SelectFilter, TimeRangeFilter, formatTimestamp, eventTypeBadge, LiveRefreshToggle } from '@samidarko/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { useLiveRefresh } from '../hooks/useLiveRefresh.ts'
import { Search } from 'lucide-react'

const EVENT_TYPE_OPTIONS = [
  { label: 'All Event Types', value: '' },
  { label: 'Message', value: 'message' },
  { label: 'Audit', value: 'audit' },
  { label: 'Task', value: 'task' },
]

export default function Timeline() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()
  const [traceSearch, setTraceSearch] = useState(searchParams.get('trace_id') ?? '')

  const filters: TimelineFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    event_type: searchParams.get('event_type') ?? undefined,
    trace_id: searchParams.get('trace_id') ?? undefined,
    from: searchParams.get('from') ?? undefined,
    to: searchParams.get('to') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const isDefaultView =
    !filters.agent_id &&
    !filters.event_type &&
    !filters.trace_id &&
    !filters.session_id &&
    !filters.from &&
    !filters.to &&
    (filters.page ?? 1) === 1

  const { isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
    defaultEnabled: true,
    interval: 5_000,
    isDefaultView,
  })

  const { data, isLoading, error, refetch } = useTimeline(filters, true, refetchInterval)
  const { data: agents } = useAgents()

  function handleTraceSearch() {
    updateFilter('trace_id', traceSearch.trim())
  }

  return (
    <div>
      {/* Header */}
      <div className="flex items-start justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Unified Event Timeline</h2>
          <p className="text-sm text-muted/60 mt-1">
            Monitor live events across Messages, Audit Log, and Tasks
          </p>
        </div>
        <LiveRefreshToggle isLive={isEffectivelyLive} onToggle={toggle} />
      </div>

      {/* Filter bar */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <div className="relative flex-1 min-w-[200px]">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted/40" />
            <input
              type="text"
              aria-label="Search events by trace ID, summary, or payload"
              placeholder="Search Trace ID, Event Summary, or Payload..."
              value={traceSearch}
              onChange={(e) => setTraceSearch(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleTraceSearch()}
              className="w-full bg-bg border border-white/[0.06] rounded-lg pl-9 pr-3 py-2 text-sm text-muted placeholder:text-muted/30 focus:outline-none focus:border-accent/40 font-mono"
            />
          </div>
          <AgentFilter
            agents={agents}
            value={filters.agent_id ?? ''}
            onChange={(v) => updateFilter('agent_id', v)}
          />
          <SelectFilter
            ariaLabel="Filter by event type"
            value={filters.event_type ?? ''}
            onChange={(v) => updateFilter('event_type', v)}
            options={EVENT_TYPE_OPTIONS}
          />
          <button
            onClick={handleTraceSearch}
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-light transition-colors"
          >
            <Search size={14} />
            Search
          </button>
          <TimeRangeFilter
            value={{ from: filters.from, to: filters.to }}
            onChange={(range) => {
              updateFilter('from', range.from ?? '')
              updateFilter('to', range.to ?? '')
            }}
          />
          {(filters.agent_id || filters.event_type || filters.trace_id || filters.from || filters.to) && (
            <button
              onClick={() => {
                setTraceSearch('')
                setSearchParams(new URLSearchParams())
              }}
              className="text-xs text-muted/60 hover:text-muted transition-colors"
            >
              Clear All
            </button>
          )}
        </div>
      </div>

      {/* Table */}
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState
          message="No events match your filters"
          action={(filters.agent_id || filters.event_type || filters.trace_id || filters.from || filters.to)
            ? { label: 'Clear filters', onClick: () => { setTraceSearch(''); setSearchParams(new URLSearchParams()) } }
            : undefined}
        />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                  <th className="text-left px-4 py-3 font-medium">Agent</th>
                  <th className="text-left px-4 py-3 font-medium">Subsystem</th>
                  <th className="text-left px-4 py-3 font-medium">Summary</th>
                  <th className="text-left px-4 py-3 font-medium">Trace ID</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((row, i) => {
                  const badge = eventTypeBadge(row.event_type)
                  return (
                    <ListRow key={i} variant="static">
                      <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                        {formatTimestamp(row.created_at)}
                      </td>
                      <td className="px-4 py-3 text-heading text-xs font-medium">
                        {row.agent_id ?? '—'}
                      </td>
                      <td className="px-4 py-3">
                        <span
                          className={`inline-flex items-center gap-1.5 text-xs font-medium px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
                        >
                          <span className={`w-1.5 h-1.5 rounded-full ${badge.dot}`} />
                          {badge.label}
                        </span>
                      </td>
                      <td className="px-4 py-3 text-muted/80 text-xs max-w-md truncate">
                        {row.event_subtype && (
                          <span className="text-muted/50 mr-1.5">{row.event_subtype}</span>
                        )}
                        {row.summary}
                      </td>
                      <td className="px-4 py-3">
                        {row.trace_id ? (
                          <Link
                            to={`/traces/${row.trace_id}`}
                            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                          >
                            {row.trace_id}
                          </Link>
                        ) : (
                          <span className="text-muted/30 text-xs">-</span>
                        )}
                      </td>
                    </ListRow>
                  )
                })}
              </tbody>
            </table>
          </div>
          <div className="flex items-center justify-center mt-4">
            <Pagination
              page={filters.page ?? 1}
              perPage={filters.per_page ?? 50}
              total={data.total}
              onPageChange={setPage}
            />
          </div>
        </>
      )}
    </div>
  )
}
