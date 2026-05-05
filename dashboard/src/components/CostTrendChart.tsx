import { useMemo } from 'react'
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from 'recharts'
import { LoadingState, ErrorState, EmptyState, formatApiError } from '@senara-solutions/ui'

export interface CostTrendBucket {
  timestamp: string
  cost_usd: number
  input_tokens: number
  output_tokens: number
  call_count: number
  agent_id: string | null
}

export type ChartVariant = 'total' | 'agent'

interface CostTrendChartProps {
  data: CostTrendBucket[] | undefined
  variant: ChartVariant
  onVariantChange: (variant: ChartVariant) => void
  bucketSize: string
  isLoading: boolean
  error: Error | null
  onRetry: () => void
  hasEstimatedPricing?: boolean
  defaultRange?: string
}

/**
 * Deterministic hex color palette for chart series.
 * Mirrors the hue order of agentColors.ts (blue, emerald, purple, amber, pink,
 * cyan, orange, indigo, rose, teal) but as hex values for recharts fills.
 */
const CHART_PALETTE = [
  '#60a5fa', // blue-400
  '#34d399', // emerald-400
  '#a78bfa', // purple-400
  '#fbbf24', // amber-400
  '#f472b6', // pink-400
  '#22d3ee', // cyan-400
  '#fb923c', // orange-400
  '#818cf8', // indigo-400
  '#fb7185', // rose-400
  '#2dd4bf', // teal-400
]

function hashString(str: string): number {
  let hash = 0
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i)
    hash = ((hash << 5) - hash + char) | 0
  }
  return Math.abs(hash)
}

function getAgentHex(agentName: string): string {
  return CHART_PALETTE[hashString(agentName) % CHART_PALETTE.length]
}

/** Primary accent color for total cost line. */
const ACCENT = '#7c6af7'

function formatXAxis(ts: string, bucketSize: string): string {
  try {
    const d = new Date(ts)
    if (bucketSize === 'hour') {
      return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
    }
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  } catch {
    return ts.slice(0, 10)
  }
}

function formatCost(v: number): string {
  if (v >= 1) return `$${v.toFixed(2)}`
  if (v >= 0.01) return `$${v.toFixed(3)}`
  if (v >= 0.001) return `$${v.toFixed(4)}`
  if (v === 0) return '$0.00'
  return `$${v.toExponential(1)}`
}

function formatTokensCompact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

interface TooltipEntry {
  name: string
  value: number
  color: string
}

function CustomTooltip({ active, payload, label, bucketSize }: {
  active?: boolean
  payload?: TooltipEntry[]
  label?: string
  bucketSize: string
}) {
  if (!active || !payload?.length) return null

  const totalCost = payload.reduce((sum, entry) => sum + (entry.value || 0), 0)

  return (
    <div className="bg-bg-card border border-white/[0.08] rounded-lg px-3 py-2 shadow-lg">
      <p className="text-xs text-muted/60 mb-1">
        {label ? formatXAxis(String(label), bucketSize) : ''}
      </p>
      {payload.length === 1 ? (
        <p className="text-sm text-heading font-medium">{formatCost(payload[0].value)}</p>
      ) : (
        <>
          <p className="text-sm text-heading font-medium mb-1">{formatCost(totalCost)} total</p>
          <div className="space-y-0.5">
            {payload.map((entry) => (
              <div key={entry.name} className="flex items-center gap-2 text-xs">
                <span
                  className="w-2 h-2 rounded-full shrink-0"
                  style={{ backgroundColor: entry.color }}
                />
                <span className="text-muted">{entry.name}</span>
                <span className="text-muted/60 ml-auto">{formatCost(entry.value)}</span>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  )
}

export default function CostTrendChart({
  data,
  variant,
  onVariantChange,
  bucketSize,
  isLoading,
  error,
  onRetry,
  hasEstimatedPricing,
  defaultRange,
}: CostTrendChartProps) {

  // Transform data for recharts
  const { chartData, agentIds } = useMemo(() => {
    if (!data?.length) return { chartData: [], agentIds: [] as string[] }

    if (variant === 'total') {
      // Aggregate across agents per timestamp
      const byTimestamp = new Map<string, { cost_usd: number; input_tokens: number; output_tokens: number; call_count: number }>()
      for (const b of data) {
        const existing = byTimestamp.get(b.timestamp)
        if (existing) {
          existing.cost_usd += b.cost_usd
          existing.input_tokens += b.input_tokens
          existing.output_tokens += b.output_tokens
          existing.call_count += b.call_count
        } else {
          byTimestamp.set(b.timestamp, {
            cost_usd: b.cost_usd,
            input_tokens: b.input_tokens,
            output_tokens: b.output_tokens,
            call_count: b.call_count,
          })
        }
      }
      return {
        chartData: Array.from(byTimestamp.entries()).map(([ts, v]) => ({
          timestamp: ts,
          cost_usd: v.cost_usd,
          input_tokens: v.input_tokens,
          output_tokens: v.output_tokens,
          call_count: v.call_count,
        })),
        agentIds: [] as string[],
      }
    }

    // Stacked by agent: pivot to { timestamp, agent1: cost, agent2: cost, ... }
    const agents = new Set<string>()
    const byTimestamp = new Map<string, Record<string, number>>()
    for (const b of data) {
      const agentKey = b.agent_id ?? 'unknown'
      agents.add(agentKey)
      const existing = byTimestamp.get(b.timestamp) ?? {}
      existing[agentKey] = (existing[agentKey] ?? 0) + b.cost_usd
      byTimestamp.set(b.timestamp, existing)
    }

    const sortedAgents = Array.from(agents).sort()
    return {
      chartData: Array.from(byTimestamp.entries()).map(([ts, costs]) => ({
        timestamp: ts,
        ...costs,
      })),
      agentIds: sortedAgents,
    }
  }, [data, variant])

  const hasData = chartData.length > 0

  return (
    <div className="bg-bg-card rounded-xl p-6 mb-4" role="img" aria-label={`Cost trend chart showing ${bucketSize}ly cost data`}>
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h3 className="text-heading text-sm font-semibold">Cost Trend</h3>
          {defaultRange && (
            <p className="text-[10px] text-muted/40 mt-0.5">Showing {defaultRange}</p>
          )}
        </div>
        <div className="flex items-center gap-1 bg-bg rounded-lg p-0.5">
          <button
            onClick={() => onVariantChange('total')}
            className={`px-3 py-1 rounded-md text-xs transition-colors ${
              variant === 'total'
                ? 'bg-accent/15 text-accent-light font-medium'
                : 'text-muted/60 hover:text-muted'
            }`}
          >
            Total
          </button>
          <button
            onClick={() => onVariantChange('agent')}
            className={`px-3 py-1 rounded-md text-xs transition-colors ${
              variant === 'agent'
                ? 'bg-accent/15 text-accent-light font-medium'
                : 'text-muted/60 hover:text-muted'
            }`}
          >
            By Agent
          </button>
        </div>
      </div>

      {/* Chart body */}
      {isLoading ? (
        <div className="h-[200px] flex items-center justify-center">
          <LoadingState variant="list" rows={3} />
        </div>
      ) : error ? (
        <div className="h-[200px] flex items-center justify-center">
          <ErrorState message={formatApiError(error)} retry={onRetry} />
        </div>
      ) : !hasData ? (
        <div className="h-[200px] flex items-center justify-center">
          <EmptyState message="No cost data for this time range" />
        </div>
      ) : (
        <ResponsiveContainer width="100%" height={200}>
          <AreaChart data={chartData} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
            <defs>
              {variant === 'total' ? (
                <linearGradient id="costGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={ACCENT} stopOpacity={0.3} />
                  <stop offset="100%" stopColor={ACCENT} stopOpacity={0} />
                </linearGradient>
              ) : (
                agentIds.map((id) => (
                  <linearGradient key={id} id={`gradient-${id}`} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={getAgentHex(id)} stopOpacity={0.3} />
                    <stop offset="100%" stopColor={getAgentHex(id)} stopOpacity={0.05} />
                  </linearGradient>
                ))
              )}
            </defs>
            <XAxis
              dataKey="timestamp"
              tickFormatter={(v) => formatXAxis(v, bucketSize)}
              tick={{ fill: '#a0a8b8', fontSize: 10 }}
              axisLine={false}
              tickLine={false}
              minTickGap={40}
            />
            <YAxis
              tickFormatter={(v) => formatCost(v)}
              tick={{ fill: '#a0a8b8', fontSize: 10 }}
              axisLine={false}
              tickLine={false}
              width={55}
            />
            <Tooltip
              content={<CustomTooltip bucketSize={bucketSize} />}
              cursor={{ stroke: '#a0a8b8', strokeOpacity: 0.2 }}
            />
            {variant === 'total' ? (
              <Area
                type="monotone"
                dataKey="cost_usd"
                stroke={ACCENT}
                strokeWidth={2}
                fill="url(#costGradient)"
                dot={chartData.length === 1}
                name="Cost"
              />
            ) : (
              agentIds.map((id) => (
                <Area
                  key={id}
                  type="monotone"
                  dataKey={id}
                  stackId="agents"
                  stroke={getAgentHex(id)}
                  strokeWidth={1.5}
                  fill={`url(#gradient-${id})`}
                  name={id}
                />
              ))
            )}
            {variant === 'agent' && agentIds.length > 1 && (
              <Legend
                iconType="circle"
                iconSize={8}
                wrapperStyle={{ fontSize: 11, color: '#a0a8b8', paddingTop: 8 }}
              />
            )}
          </AreaChart>
        </ResponsiveContainer>
      )}

      {/* Footnotes */}
      {hasEstimatedPricing && hasData && (
        <p className="text-[10px] text-muted/30 mt-2">
          Cost is estimated from token counts. Some models use approximate pricing.
        </p>
      )}

      {/* Screen reader data table */}
      {hasData && (
        <table className="sr-only" aria-label="Cost trend data">
          <thead>
            <tr>
              <th>Time</th>
              {variant === 'total' ? (
                <>
                  <th>Cost (USD)</th>
                  <th>Input tokens</th>
                  <th>Output tokens</th>
                  <th>Calls</th>
                </>
              ) : (
                agentIds.map((id) => <th key={id}>{id} cost (USD)</th>)
              )}
            </tr>
          </thead>
          <tbody>
            {chartData.map((row, i) => {
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
              const r = row as any
              return (
                <tr key={i}>
                  <td>{row.timestamp}</td>
                  {variant === 'total' ? (
                    <>
                      <td>{formatCost(r.cost_usd ?? 0)}</td>
                      <td>{formatTokensCompact(r.input_tokens ?? 0)}</td>
                      <td>{formatTokensCompact(r.output_tokens ?? 0)}</td>
                      <td>{r.call_count ?? 0}</td>
                    </>
                  ) : (
                    agentIds.map((id) => (
                      <td key={id}>{formatCost(r[id] ?? 0)}</td>
                    ))
                  )}
                </tr>
              )
            })}
          </tbody>
        </table>
      )}
    </div>
  )
}
