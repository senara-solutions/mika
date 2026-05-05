import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import LlmCalls from './LlmCalls.tsx'

// Mock recharts to avoid rendering issues in jsdom
vi.mock('recharts', () => ({
  AreaChart: ({ children }: { children: React.ReactNode }) => <div data-testid="area-chart">{children}</div>,
  Area: ({ dataKey }: { dataKey: string }) => <div data-testid={`area-${dataKey}`} />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Legend: () => <div data-testid="legend" />,
  ResponsiveContainer: ({ children }: { children: React.ReactNode }) => <div data-testid="responsive-container">{children}</div>,
}))

const mockCostTrendData = {
  buckets: [
    { timestamp: '2026-05-05T10:00:00Z', cost_usd: 0.42, input_tokens: 150000, output_tokens: 8000, call_count: 5, agent_id: 'mika-dev' },
    { timestamp: '2026-05-05T11:00:00Z', cost_usd: 0.35, input_tokens: 120000, output_tokens: 6000, call_count: 4, agent_id: 'mika-dev' },
  ],
  bucket_size: 'hour',
  has_estimated_pricing: false,
  estimated_models: [],
}

const mockLlmCallsData = {
  data: [],
  total: 0,
  page: 1,
  per_page: 50,
}

// Track which query keys were used
const queryFnCalls: Array<{ queryKey: unknown[] }> = []

vi.mock('../api/llmCalls.ts', () => ({
  useLlmCalls: () => ({
    data: mockLlmCallsData,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
  useCostTrend: (filters: Record<string, unknown>) => {
    queryFnCalls.push({ queryKey: ['cost-trend', filters] })
    return {
      data: mockCostTrendData,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    }
  },
}))

vi.mock('../api/agents.ts', () => ({
  useAgents: () => ({ data: ['mika-dev', 'mika-qa'] }),
}))

function renderPage(initialEntries: string[] = ['/llm-calls']) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={initialEntries}>
        <LlmCalls />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('LlmCalls page — CostTrendChart integration', () => {
  beforeEach(() => {
    queryFnCalls.length = 0
  })

  it('renders the cost trend chart on the page', () => {
    renderPage()
    expect(screen.getByText('Cost Trend')).toBeTruthy()
    expect(screen.getByTestId('area-chart')).toBeTruthy()
  })

  it('defaults to total variant when no chart URL param', () => {
    renderPage()
    // Total variant renders a single area with dataKey="cost_usd"
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()
  })

  it('reads agent variant from chart URL param', () => {
    renderPage(['/llm-calls?chart=agent'])
    // Agent variant renders per-agent areas
    expect(screen.getByTestId('area-mika-dev')).toBeTruthy()
  })

  it('variant toggle updates URL param', () => {
    renderPage()
    // Initially total
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()

    // Click "By Agent" — switches to agent variant
    fireEvent.click(screen.getByText('By Agent'))
    expect(screen.getByTestId('area-mika-dev')).toBeTruthy()

    // Click "Total" — switches back
    fireEvent.click(screen.getByText('Total'))
    expect(screen.getByTestId('area-cost_usd')).toBeTruthy()
  })

  it('propagates filter state from page to chart', () => {
    renderPage(['/llm-calls?agent_id=mika-dev&from=2026-05-01T00:00:00Z&to=2026-05-05T00:00:00Z'])
    // The useCostTrend mock captures the filters passed to it
    const costCall = queryFnCalls.find(c =>
      JSON.stringify(c.queryKey).includes('cost-trend'),
    )
    expect(costCall).toBeTruthy()
    const filters = costCall!.queryKey[1] as Record<string, unknown>
    expect(filters.agent_id).toBe('mika-dev')
    expect(filters.from).toBe('2026-05-01T00:00:00Z')
    expect(filters.to).toBe('2026-05-05T00:00:00Z')
  })

  it('propagates undefined filters when no URL params set', () => {
    renderPage()
    const costCall = queryFnCalls.find(c =>
      JSON.stringify(c.queryKey).includes('cost-trend'),
    )
    expect(costCall).toBeTruthy()
    const filters = costCall!.queryKey[1] as Record<string, unknown>
    expect(filters.agent_id).toBeUndefined()
    expect(filters.from).toBeUndefined()
    expect(filters.to).toBeUndefined()
  })

  it('shows default range label when no from param', () => {
    renderPage()
    expect(screen.getByText('Showing last 24 hours')).toBeTruthy()
  })

  it('does not show default range label when from param is set', () => {
    renderPage(['/llm-calls?from=2026-05-01T00:00:00Z'])
    expect(screen.queryByText('Showing last 24 hours')).toBeNull()
  })
})
