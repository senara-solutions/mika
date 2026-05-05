import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import CostTrendChart, { type CostTrendBucket, type ChartVariant } from './CostTrendChart.tsx'

// Mock recharts to avoid rendering issues in jsdom
vi.mock('recharts', () => ({
  AreaChart: ({ children }: { children: React.ReactNode }) => <div data-testid="area-chart">{children}</div>,
  Area: ({ dataKey, name }: { dataKey: string; name?: string }) => <div data-testid={`area-${dataKey}`} data-name={name} />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Legend: () => <div data-testid="legend" />,
  ResponsiveContainer: ({ children }: { children: React.ReactNode }) => <div data-testid="responsive-container">{children}</div>,
}))

const mockBuckets: CostTrendBucket[] = [
  { timestamp: '2026-05-05T10:00:00Z', cost_usd: 0.42, input_tokens: 150000, output_tokens: 8000, call_count: 5, agent_id: 'mika-dev' },
  { timestamp: '2026-05-05T10:00:00Z', cost_usd: 0.07, input_tokens: 20000, output_tokens: 2000, call_count: 2, agent_id: 'mika-qa' },
  { timestamp: '2026-05-05T11:00:00Z', cost_usd: 0.35, input_tokens: 120000, output_tokens: 6000, call_count: 4, agent_id: 'mika-dev' },
]

function renderChart(overrides: Partial<{
  data: CostTrendBucket[] | undefined
  variant: ChartVariant
  onVariantChange: (v: ChartVariant) => void
  bucketSize: string
  isLoading: boolean
  error: Error | null
  onRetry: () => void
  hasEstimatedPricing: boolean
  defaultRange: string
}> = {}) {
  const defaults = {
    data: mockBuckets,
    variant: 'total' as ChartVariant,
    onVariantChange: vi.fn(),
    bucketSize: 'hour',
    isLoading: false,
    error: null,
    onRetry: vi.fn(),
    ...overrides,
  }
  return { ...render(<CostTrendChart {...defaults} />), ...defaults }
}

describe('CostTrendChart', () => {
  it('renders chart with data in total variant', () => {
    renderChart()
    expect(screen.getByTestId('area-chart')).toBeTruthy()
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()
    expect(screen.getByText('Cost Trend')).toBeTruthy()
  })

  it('renders stacked areas per agent in agent variant', () => {
    renderChart({ variant: 'agent' })
    expect(screen.getByTestId('area-mika-dev')).toBeTruthy()
    expect(screen.getByTestId('area-mika-qa')).toBeTruthy()
  })

  it('renders EmptyState when data is empty', () => {
    renderChart({ data: [] })
    expect(screen.getByText('No cost data for this time range')).toBeTruthy()
  })

  it('renders LoadingState when loading', () => {
    renderChart({ data: undefined, isLoading: true })
    expect(screen.getByLabelText(/loading/i)).toBeTruthy()
  })

  it('renders ErrorState with retry on error', () => {
    const onRetry = vi.fn()
    renderChart({ data: undefined, error: new Error('Network error'), onRetry })
    const retryButton = screen.getByText('Retry')
    expect(retryButton).toBeTruthy()
    fireEvent.click(retryButton)
    expect(onRetry).toHaveBeenCalled()
  })

  it('renders single bucket without crash', () => {
    const single: CostTrendBucket[] = [
      { timestamp: '2026-05-05T10:00:00Z', cost_usd: 0.42, input_tokens: 150000, output_tokens: 8000, call_count: 5, agent_id: 'mika-dev' },
    ]
    renderChart({ data: single })
    expect(screen.getByTestId('area-chart')).toBeTruthy()
  })

  it('calls onVariantChange when toggling variants', () => {
    const onVariantChange = vi.fn()
    renderChart({ variant: 'total', onVariantChange })

    fireEvent.click(screen.getByText('By Agent'))
    expect(onVariantChange).toHaveBeenCalledWith('agent')

    fireEvent.click(screen.getByText('Total'))
    expect(onVariantChange).toHaveBeenCalledWith('total')
  })

  it('shows estimated pricing footnote', () => {
    renderChart({ hasEstimatedPricing: true })
    expect(screen.getByText(/estimated from token counts/i)).toBeTruthy()
  })

  it('shows default range label when provided', () => {
    renderChart({ defaultRange: 'last 24 hours' })
    expect(screen.getByText('Showing last 24 hours')).toBeTruthy()
  })

  it('renders accessible data table for screen readers', () => {
    renderChart()
    expect(screen.getByLabelText('Cost trend data')).toBeTruthy()
  })
})
