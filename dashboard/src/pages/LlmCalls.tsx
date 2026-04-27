import { Link } from 'react-router'
import { useLlmCalls, type LlmCallsFilters } from '../api/llmCalls.ts'
import { useAgents } from '../api/agents.ts'
import { Pagination, EmptyState, StatusBadge, formatTimestamp } from '@senara-solutions/ui'
import type { StatusBadgeVariant } from '@senara-solutions/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { Search } from 'lucide-react'

function llmStatusVariant(status: string): { variant: StatusBadgeVariant; label: string } {
  switch (status) {
    case 'success': return { variant: 'success', label: 'Success' }
    case 'error': return { variant: 'error', label: 'Error' }
    default: return { variant: 'neutral', label: status }
  }
}

function formatTokens(n: number | null): string {
  if (n == null) return '-'
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${ms}ms`
}

export default function LlmCalls() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()

  const filters: LlmCallsFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    model: searchParams.get('model') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useLlmCalls(filters)
  const { data: agents } = useAgents()

  return (
    <div>
      {/* Header */}
      <div className="flex items-start justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">LLM Calls</h2>
          <p className="text-sm text-muted/60 mt-1">
            {data ? `${data.total} LLM call${data.total !== 1 ? 's' : ''} recorded` : 'Loading LLM calls...'}
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
              aria-label="Search by model name"
              placeholder="Filter by model..."
              value={searchParams.get('model') ?? ''}
              onChange={(e) => updateFilter('model', e.target.value)}
              className="w-full bg-bg border border-white/[0.06] rounded-lg pl-9 pr-3 py-2 text-sm text-muted placeholder:text-muted/30 focus:outline-none focus:border-accent/40 font-mono"
            />
          </div>
          <select
            value={filters.agent_id ?? ''}
            onChange={(e) => updateFilter('agent_id', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
          >
            <option value="">All Agents</option>
            {agents?.map((a) => (
              <option key={a.name} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
          {(filters.agent_id || filters.model) && (
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
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No LLM calls match your filters" />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="w-8 px-2 py-3" />
                  <th className="text-left px-4 py-3 font-medium">Timestamp</th>
                  <th className="text-left px-4 py-3 font-medium">Provider</th>
                  <th className="text-left px-4 py-3 font-medium">Model</th>
                  <th className="text-right px-4 py-3 font-medium">Input</th>
                  <th className="text-right px-4 py-3 font-medium">Output</th>
                  <th className="text-right px-4 py-3 font-medium">Cache R</th>
                  <th className="text-right px-4 py-3 font-medium">Latency</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-left px-4 py-3 font-medium">Trace</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((row) => (
                  <tr key={row.id} className="hover:bg-white/[0.02] transition-colors">
                    <td className="px-2 py-3">
                      <Link
                        to={`/llm-calls/${row.id}`}
                        className="text-accent/40 hover:text-accent transition-colors text-xs"
                        title="View details"
                      >
                        &rarr;
                      </Link>
                    </td>
                    <td className="px-4 py-3 text-muted/70 whitespace-nowrap font-mono text-xs">
                      {formatTimestamp(row.created_at)}
                    </td>
                    <td className="px-4 py-3 text-xs text-heading font-medium">
                      {row.provider}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted font-mono max-w-[200px] truncate">
                      {row.model}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right">
                      {formatTokens(row.input_tokens)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right">
                      {formatTokens(row.output_tokens)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/40 font-mono text-right">
                      {formatTokens(row.cache_read_tokens)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono text-right whitespace-nowrap">
                      {formatLatency(row.latency_ms)}
                    </td>
                    <td className="px-4 py-3">
                      <StatusBadge {...llmStatusVariant(row.status)} />
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
                  </tr>
                ))}
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
