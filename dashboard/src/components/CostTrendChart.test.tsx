import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import CostTrendChart, { type CostTrendBucket } from './CostTrendChart.tsx'

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

describe('CostTrendChart', () => {
  it('renders chart with data in total variant', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    expect(screen.getByTestId('area-chart')).toBeTruthy()
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()
    expect(screen.getByText('Cost Trend')).toBeTruthy()
  })

  it('renders stacked areas per agent in agent variant', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    // Switch to agent variant
    fireEvent.click(screen.getByText('By Agent'))
    expect(screen.getByTestId('area-mika-dev')).toBeTruthy()
    expect(screen.getByTestId('area-mika-qa')).toBeTruthy()
  })

  it('renders EmptyState when data is empty', () => {
    render(
      <CostTrendChart
        data={[]}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    expect(screen.getByText('No cost data for this time range')).toBeTruthy()
  })

  it('renders LoadingState when loading', () => {
    render(
      <CostTrendChart
        data={undefined}
        bucketSize="hour"
        isLoading={true}
        error={null}
        onRetry={() => {}}
      />
    )
    // LoadingState renders skeleton elements with aria-label
    expect(screen.getByLabelText(/loading/i)).toBeTruthy()
  })

  it('renders ErrorState with retry on error', () => {
    const onRetry = vi.fn()
    render(
      <CostTrendChart
        data={undefined}
        bucketSize="hour"
        isLoading={false}
        error={new Error('Network error')}
        onRetry={onRetry}
      />
    )
    const retryButton = screen.getByText('Retry')
    expect(retryButton).toBeTruthy()
    fireEvent.click(retryButton)
    expect(onRetry).toHaveBeenCalled()
  })

  it('renders single bucket without crash', () => {
    const single: CostTrendBucket[] = [
      { timestamp: '2026-05-05T10:00:00Z', cost_usd: 0.42, input_tokens: 150000, output_tokens: 8000, call_count: 5, agent_id: 'mika-dev' },
    ]
    render(
      <CostTrendChart
        data={single}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    expect(screen.getByTestId('area-chart')).toBeTruthy()
  })

  it('toggles between total and agent variants', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    // Initially total mode
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()

    // Switch to agent
    fireEvent.click(screen.getByText('By Agent'))
    expect(screen.getByTestId('area-mika-dev')).toBeTruthy()

    // Switch back to total
    fireEvent.click(screen.getByText('Total'))
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()
  })

  it('shows estimated pricing footnote', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
        hasEstimatedPricing={true}
      />
    )
    expect(screen.getByText(/estimated from token counts/i)).toBeTruthy()
  })

  it('shows default range label when provided', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
        defaultRange="last 24 hours"
      />
    )
    expect(screen.getByText('Showing last 24 hours')).toBeTruthy()
  })

  it('renders accessible data table for screen readers', () => {
    render(
      <CostTrendChart
        data={mockBuckets}
        bucketSize="hour"
        isLoading={false}
        error={null}
        onRetry={() => {}}
      />
    )
    expect(screen.getByLabelText('Cost trend data')).toBeTruthy()
  })
})
