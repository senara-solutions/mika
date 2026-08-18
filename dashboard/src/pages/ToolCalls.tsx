import { useState, Fragment } from 'react'
import { Link } from 'react-router'
import { useToolCalls, type ToolCallsFilters } from '../api/toolCalls.ts'
import { useAgents } from '../api/agents.ts'
import { CopyButton, Pagination, EmptyState, LoadingState, ErrorState, formatApiError, StatusBadge, ListRow, AgentFilter, SelectFilter, TimeRangeFilter, formatTimestamp } from '@samidarko/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { Search } from 'lucide-react'

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${ms}ms`
}

function sourceBadge(source: string) {
  switch (source) {
    case 'builtin':
      return 'bg-accent/15 text-accent/80'
    case 'skill':
      return 'bg-purple-400/15 text-purple-400'
    case 'mcp':
      return 'bg-amber-400/15 text-amber-400'
    default:
      return 'bg-white/[0.06] text-muted/60'
  }
}

const SUCCESS_OPTIONS = [
  { label: 'All Results', value: '' },
  { label: 'Success', value: 'true' },
  { label: 'Failed', value: 'false' },
]

export default function ToolCalls() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  const filters: ToolCallsFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    tool_name: searchParams.get('tool_name') ?? undefined,
    success: searchParams.get('success') ?? undefined,
    from: searchParams.get('from') ?? undefined,
    to: searchParams.get('to') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error, refetch } = useToolCalls(filters)
  const { data: agents } = useAgents()

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) { next.delete(id) } else { next.add(id) }
      return next
    })
  }

  return (
    <div>
      {/* Header */}
      <div className="flex items-start justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Tool Calls</h2>
          <p className="text-sm text-muted/60 mt-1">
            {data ? `${data.total} tool call${data.total !== 1 ? 's' : ''} recorded` : 'Loading tool calls...'}
          </p>
        </div>
      </div>

      {/* Filter bar */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <div className="relative flex-1 min-w-[200px]">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted/40" />
            <input
              type="text"
              aria-label="Search by tool name"
              placeholder="Filter by tool name..."
              value={searchParams.get('tool_name') ?? ''}
              onChange={(e) => updateFilter('tool_name', e.target.value)}
              className="w-full bg-bg border border-white/[0.06] rounded-lg pl-9 pr-3 py-2 text-sm text-muted placeholder:text-muted/30 focus:outline-none focus:border-accent/40 font-mono"
            />
          </div>
          <AgentFilter
            agents={agents}
            value={filters.agent_id ?? ''}
            onChange={(v) => updateFilter('agent_id', v)}
          />
          <SelectFilter
            ariaLabel="Filter by result"
            value={filters.success ?? ''}
            onChange={(v) => updateFilter('success', v)}
            options={SUCCESS_OPTIONS}
          />
          <TimeRangeFilter
            value={{ from: filters.from, to: filters.to }}
            onChange={(range) => {
              updateFilter('from', range.from ?? '')
              updateFilter('to', range.to ?? '')
            }}
          />
          {(filters.agent_id || filters.tool_name || filters.success || filters.from || filters.to) && (
            <button
              onClick={() => setSearchParams(new URLSearchParams())}
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
          message="No tool calls match your filters"
          action={(filters.agent_id || filters.tool_name || filters.success || filters.from || filters.to)
            ? { label: 'Clear filters', onClick: () => setSearchParams(new URLSearchParams()) }
            : undefined}
        />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="w-8 px-2 py-3" />
                  <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                  <th className="text-left px-4 py-3 font-medium">Tool</th>
                  <th className="text-left px-4 py-3 font-medium">Source</th>
                  <th className="text-left px-4 py-3 font-medium">Skill</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-right px-4 py-3 font-medium">Latency</th>
                  <th className="text-left px-4 py-3 font-medium">Trace</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((row) => {
                  const isOpen = expanded.has(row.id)
                  return (
                    <Fragment key={row.id}>
                      <ListRow
                        variant="expandable"
                        isExpanded={isOpen}
                        onToggle={() => toggleExpand(row.id)}
                        ariaLabel={`Toggle details for tool call ${row.tool_name}`}
                      >
                        <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                          {formatTimestamp(row.created_at)}
                        </td>
                        <td className="px-4 py-3 text-xs font-mono font-medium max-w-[180px] truncate">
                          <Link
                            to={`/tool-calls/${row.id}`}
                            className="text-accent hover:text-accent-light transition-colors"
                          >
                            {row.tool_name}
                          </Link>
                        </td>
                        <td className="px-4 py-3">
                          <span className={`inline-flex items-center text-[10px] font-semibold px-2 py-0.5 rounded-full ${sourceBadge(row.tool_source)}`}>
                            {row.tool_source}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-xs text-muted/60">
                          {row.skill_name ?? <span className="text-muted/30">-</span>}
                        </td>
                        <td className="px-4 py-3">
                          <StatusBadge variant={row.success ? 'success' : 'error'} label={row.success ? 'Ok' : 'Fail'} />
                        </td>
                        <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right whitespace-nowrap">
                          {formatLatency(row.latency_ms)}
                        </td>
                        <td className="px-4 py-3">
                          {row.trace_id ? (
                            <Link
                              to={`/traces/${row.trace_id}`}
                              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                            >
                              {row.trace_id.slice(0, 8)}...
                            </Link>
                          ) : (
                            <span className="text-muted/30 text-xs">-</span>
                          )}
                        </td>
                      </ListRow>
                      {isOpen && (
                        <tr>
                          <td colSpan={8} className="px-6 py-4 bg-white/[0.02]">
                            <div className="space-y-3">
                              {row.input && (
                                <div>
                                  <div className="flex items-center gap-2">
                                    <span className="text-[10px] text-muted/40 uppercase tracking-wider">Input</span>
                                    <CopyButton text={row.input} title="Copy input" />
                                  </div>
                                  <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                                    {row.input}
                                  </div>
                                </div>
                              )}
                              {row.output && (
                                <div>
                                  <div className="flex items-center gap-2">
                                    <span className="text-[10px] text-muted/40 uppercase tracking-wider">Output</span>
                                    <CopyButton text={row.output} title="Copy output" />
                                  </div>
                                  <div className="font-mono text-xs text-muted/70 pl-2 border-l border-white/[0.06] mt-1 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                                    {row.output}
                                  </div>
                                </div>
                              )}
                              {row.error_message && (
                                <div>
                                  <span className="text-[10px] text-red-400/60 uppercase tracking-wider">Error</span>
                                  <div className="font-mono text-xs text-red-400/80 pl-2 border-l border-red-400/20 mt-1 whitespace-pre-wrap break-all">
                                    {row.error_message}
                                  </div>
                                </div>
                              )}
                              <div className="flex items-center gap-4 text-[10px] text-muted/30">
                                <span>Step {row.step}</span>
                                {row.non_zero_exit && <span className="text-amber-400/60">non-zero exit</span>}
                              </div>
                            </div>
                          </td>
                        </tr>
                      )}
                    </Fragment>
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
