import { useState } from 'react'
import { Link, useSearchParams } from 'react-router'
import { useTimeline, type TimelineFilters } from '../api/timeline.ts'
import Pagination from '../components/Pagination.tsx'
import EmptyState from '../components/EmptyState.tsx'
import { formatTimestamp } from '../hooks/useFormatTime.ts'
import { Search } from 'lucide-react'

const EVENT_TYPES = ['', 'message', 'audit', 'task']

function eventTypeBadge(type: string): { bg: string; text: string; label: string } {
  switch (type) {
    case 'message':
      return { bg: 'bg-blue-500/15', text: 'text-blue-400', label: 'Messages' }
    case 'audit':
      return { bg: 'bg-amber-500/15', text: 'text-amber-400', label: 'Audit Log' }
    case 'task':
      return { bg: 'bg-emerald-500/15', text: 'text-emerald-400', label: 'Tasks' }
    default:
      return { bg: 'bg-white/[0.05]', text: 'text-muted', label: type }
  }
}

export default function Timeline() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [traceSearch, setTraceSearch] = useState(searchParams.get('trace_id') ?? '')
  const [autoRefresh, setAutoRefresh] = useState(true)

  const filters: TimelineFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    event_type: searchParams.get('event_type') ?? undefined,
    trace_id: searchParams.get('trace_id') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useTimeline(filters, true, autoRefresh)

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
      {/* Header */}
      <div className="flex items-start justify-between mb-5">
        <div>
          <div className="flex items-center gap-3">
            <h2 className="text-heading text-xl font-semibold">Unified Event Timeline</h2>
            {autoRefresh && (
              <span className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase bg-emerald-500/15 text-emerald-400">
                <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
                Live
              </span>
            )}
          </div>
          <p className="text-sm text-muted/60 mt-1">
            Monitor live events across Messages, Audit Log, and Tasks
          </p>
        </div>
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-2 text-xs text-muted cursor-pointer">
            Auto-refresh
            <button
              role="switch"
              aria-checked={autoRefresh}
              onClick={() => setAutoRefresh(!autoRefresh)}
              className={`relative w-9 h-5 rounded-full transition-colors ${
                autoRefresh ? 'bg-accent' : 'bg-white/[0.1]'
              }`}
            >
              <span
                className={`absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                  autoRefresh ? 'left-[18px]' : 'left-0.5'
                }`}
              />
            </button>
          </label>
        </div>
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
          <select
            value={filters.agent_id ?? ''}
            onChange={(e) => updateFilter('agent_id', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
          >
            <option value="">All Agents</option>
          </select>
          <select
            value={filters.event_type ?? ''}
            onChange={(e) => updateFilter('event_type', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
          >
            {EVENT_TYPES.map((t) => (
              <option key={t} value={t}>
                {t ? t.charAt(0).toUpperCase() + t.slice(1) : 'All Event Types'}
              </option>
            ))}
          </select>
          <button
            onClick={handleTraceSearch}
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-light transition-colors"
          >
            <Search size={14} />
            Search
          </button>
          {(filters.agent_id || filters.event_type || filters.trace_id) && (
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
                    <tr key={i} className="hover:bg-white/[0.02] transition-colors">
                      <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                        {formatTimestamp(row.created_at)}
                      </td>
                      <td className="px-4 py-3 text-heading text-xs font-medium">
                        {row.agent_id}
                      </td>
                      <td className="px-4 py-3">
                        <span
                          className={`inline-flex items-center gap-1.5 text-xs font-medium px-2 py-0.5 rounded-full ${badge.bg} ${badge.text}`}
                        >
                          <span className={`w-1.5 h-1.5 rounded-full ${badge.text.replace('text-', 'bg-')}`} />
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
                    </tr>
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
