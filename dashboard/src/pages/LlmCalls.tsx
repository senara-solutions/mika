import { useState } from 'react'
import { Link } from 'react-router'
import { useLlmCalls, useLlmCall, useCostTrend, type LlmCallRow, type LlmCallsFilters, type CostTrendFilters } from '../api/llmCalls.ts'
import { useAgents } from '../api/agents.ts'
import { Pagination, EmptyState, LoadingState, ErrorState, formatApiError, StatusBadge, ListRow, AgentFilter, TimeRangeFilter, LiveRefreshToggle, CostMeter, formatTimestamp } from '@samidarko/ui'
import type { StatusBadgeVariant } from '@samidarko/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'
import { useLiveRefresh } from '../hooks/useLiveRefresh.ts'
import { Search } from 'lucide-react'
import CostTrendChart, { type ChartVariant } from '../components/CostTrendChart.tsx'
import LlmBodyViewer from '../components/LlmBodyViewer.tsx'

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

const COL_COUNT = 11

/** Inline detail panel shown when a row is expanded. */
function ExpandedBody({ llmCallId }: { llmCallId: string }) {
  const { data: call, isLoading, error, refetch } = useLlmCall(llmCallId)

  if (isLoading) {
    return (
      <tr>
        <td colSpan={COL_COUNT} className="px-4 py-4">
          <LoadingState variant="detail" />
        </td>
      </tr>
    )
  }

  if (error) {
    return (
      <tr>
        <td colSpan={COL_COUNT} className="px-4 py-4">
          <ErrorState message={formatApiError(error)} retry={() => refetch()} variant="detail-section" />
        </td>
      </tr>
    )
  }

  if (!call) return null

  return (
    <tr className="bg-white/[0.01]">
      <td colSpan={COL_COUNT} className="px-6 py-2">
        <LlmBodyViewer
          responseText={call.response_text}
          reasoning={call.reasoning}
          llmCallId={llmCallId}
        />
      </td>
    </tr>
  )
}

export default function LlmCalls() {
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()

  const filters: LlmCallsFilters = {
    agent_id: searchParams.get('agent_id') ?? undefined,
    model: searchParams.get('model') ?? undefined,
    from: searchParams.get('from') ?? undefined,
    to: searchParams.get('to') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const isDefaultView =
    !filters.agent_id &&
    !filters.model &&
    !filters.from &&
    !filters.to &&
    (filters.page ?? 1) === 1

  const { isLive, isEffectivelyLive, toggle, refetchInterval } = useLiveRefresh({
    defaultEnabled: false,
    interval: 15_000,
    isDefaultView,
  })

  const { data, isLoading, error, refetch } = useLlmCalls(filters, refetchInterval)
  const { data: agents } = useAgents()

  // Chart variant from URL param (default: total)
  const chartVariantParam = searchParams.get('chart')
  const chartVariant: ChartVariant = chartVariantParam === 'agent' ? 'agent' : 'total'
  const setChartVariant = (v: ChartVariant) => {
    const next = new URLSearchParams(searchParams)
    if (v === 'total') {
      next.delete('chart')
    } else {
      next.set('chart', v)
    }
    setSearchParams(next)
  }

  // Cost trend chart: the server defaults to last 24h when no `from` is provided.
  const costTrendFilters: CostTrendFilters = {
    agent_id: filters.agent_id,
    model: filters.model,
    from: filters.from,
    to: filters.to,
  }
  const { data: costTrend, isLoading: costLoading, error: costError, refetch: costRefetch } = useCostTrend(costTrendFilters, refetchInterval)

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
        <LiveRefreshToggle
          isLive={isEffectivelyLive}
          onToggle={toggle}
          disabled={!isDefaultView && isLive}
        />
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
          <AgentFilter
            agents={agents}
            value={filters.agent_id ?? ''}
            onChange={(v) => updateFilter('agent_id', v)}
          />
          <TimeRangeFilter
            value={{ from: filters.from, to: filters.to }}
            onChange={(range) => {
              updateFilter('from', range.from ?? '')
              updateFilter('to', range.to ?? '')
            }}
          />
          {(filters.agent_id || filters.model || filters.from || filters.to) && (
            <button
              onClick={() => setSearchParams(new URLSearchParams())}
              className="text-xs text-muted/60 hover:text-muted transition-colors"
            >
              Clear All
            </button>
          )}
        </div>
      </div>

      {/* Cost Trend Chart */}
      <CostTrendChart
        data={costTrend?.buckets}
        variant={chartVariant}
        onVariantChange={setChartVariant}
        bucketSize={costTrend?.bucket_size ?? 'hour'}
        isLoading={costLoading}
        error={costError}
        onRetry={() => costRefetch()}
        hasEstimatedPricing={costTrend?.has_estimated_pricing}
        defaultRange={!filters.from ? 'last 24 hours' : undefined}
      />

      {/* Table */}
      {isLoading ? (
        <LoadingState variant="list" />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !data || data.data.length === 0 ? (
        <EmptyState
          message="No LLM calls match your filters"
          action={(filters.agent_id || filters.model || filters.from || filters.to)
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
                  <th className="text-left px-4 py-3 font-medium">Provider</th>
                  <th className="text-left px-4 py-3 font-medium">Model</th>
                  <th className="text-right px-4 py-3 font-medium">Input</th>
                  <th className="text-right px-4 py-3 font-medium">Output</th>
                  <th className="text-right px-4 py-3 font-medium">Cache R</th>
                  <th className="text-right px-4 py-3 font-medium">Cost</th>
                  <th className="text-right px-4 py-3 font-medium">Latency</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-left px-4 py-3 font-medium">Trace</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((row) => {
                  const hasBody = row.has_response_text || row.has_reasoning
                  const isExpanded = expandedId === row.id

                  return hasBody ? (
                    <ExpandableRow
                      key={row.id}
                      row={row}
                      isExpanded={isExpanded}
                      onToggle={() => setExpandedId(isExpanded ? null : row.id)}
                    />
                  ) : (
                    <StaticRow key={row.id} row={row} />
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

/** Shared row cells for both expandable and static rows. */
function RowCells({ row }: { row: LlmCallRow }) {
  return (
    <>
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
      <td className="px-4 py-3 text-right">
        <CostMeter value={row.cost_usd} variant="chip" />
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
    </>
  )
}

/** Row with expand/collapse for calls that have body content. */
function ExpandableRow({ row, isExpanded, onToggle }: { row: LlmCallRow; isExpanded: boolean; onToggle: () => void }) {
  return (
    <>
      <ListRow
        variant="expandable"
        isExpanded={isExpanded}
        onToggle={onToggle}
        ariaLabel={`${isExpanded ? 'Collapse' : 'Expand'} LLM call ${row.model} response`}
      >
        <RowCells row={row} />
      </ListRow>
      {isExpanded && <ExpandedBody llmCallId={row.id} />}
    </>
  )
}

/** Static row for calls without body content (no expand affordance). */
function StaticRow({ row }: { row: LlmCallRow }) {
  return (
    <ListRow
      variant="static"
    >
      <RowCells row={row} />
    </ListRow>
  )
}
