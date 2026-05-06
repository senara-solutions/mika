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

const mockLlmCallsEmpty = {
  data: [],
  total: 0,
  page: 1,
  per_page: 50,
}

const mockLlmCallsWithBodies = {
  data: [
    {
      id: 'call-1',
      agent_id: 'mika-dev',
      session_id: 'sess-1',
      trace_id: 'trace-abc',
      provider: 'anthropic',
      model: 'claude-sonnet-4-20250514',
      input_tokens: 5000,
      output_tokens: 1200,
      cache_read_tokens: 3000,
      cache_write_tokens: null,
      latency_ms: 2500,
      stop_reason: 'end_turn',
      status: 'success',
      error_message: null,
      step: 1,
      prompt_variant: null,
      created_at: '2026-05-05T10:00:00Z',
      response_text: null,
      reasoning: null,
      has_response_text: true,
      has_reasoning: true,
      cost_usd: 0.042,
    },
    {
      id: 'call-2',
      agent_id: 'mika-dev',
      session_id: 'sess-1',
      trace_id: 'trace-abc',
      provider: 'anthropic',
      model: 'claude-sonnet-4-20250514',
      input_tokens: 2000,
      output_tokens: 500,
      cache_read_tokens: null,
      cache_write_tokens: null,
      latency_ms: 800,
      stop_reason: 'end_turn',
      status: 'success',
      error_message: null,
      step: 2,
      prompt_variant: null,
      created_at: '2026-05-05T10:01:00Z',
      response_text: null,
      reasoning: null,
      has_response_text: false,
      has_reasoning: false,
      cost_usd: 0.01,
    },
  ],
  total: 2,
  page: 1,
  per_page: 50,
}

// Mutable mock data so tests can switch between empty and populated
let currentMockData: typeof mockLlmCallsWithBodies | typeof mockLlmCallsEmpty = mockLlmCallsEmpty
let mockDetailData: Record<string, unknown> | null = null

// Track which query keys were used
const queryFnCalls: Array<{ queryKey: unknown[] }> = []

vi.mock('../api/llmCalls.ts', () => ({
  useLlmCalls: () => ({
    data: currentMockData,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
  useLlmCall: (id: string | undefined) => ({
    data: id && mockDetailData ? mockDetailData : undefined,
    isLoading: id != null && !mockDetailData,
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
    currentMockData = mockLlmCallsEmpty
    mockDetailData = null
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

describe('LlmCalls page — expandable rows', () => {
  beforeEach(() => {
    queryFnCalls.length = 0
    currentMockData = mockLlmCallsWithBodies
    mockDetailData = null
  })

  it('shows expand chevron only for rows with body content', () => {
    renderPage()
    // Row with has_response_text=true should have an expandable affordance (button role)
    const expandableRows = screen.getAllByRole('button')
    // Filter to only tr elements (ListRow expandable renders as role="button")
    const expandableTrs = expandableRows.filter(el => el.tagName === 'TR')
    expect(expandableTrs.length).toBe(1) // Only call-1 has bodies
  })

  it('renders static row for calls without bodies', () => {
    renderPage()
    // call-2 should be a static row (no role="button" or role="link")
    // The table should have 2 data rows total
    const allRows = screen.getAllByRole('row')
    // Header row + 2 data rows = 3 minimum
    expect(allRows.length).toBeGreaterThanOrEqual(3)
  })

  it('expands row on click to show loading state', () => {
    renderPage()
    // Click the expandable row
    const expandableRow = screen.getByRole('button', { name: /Expand LLM call/ })
    fireEvent.click(expandableRow)
    // Should show loading state (since mockDetailData is null, useLlmCall returns isLoading)
    // The detail panel skeleton should appear
    expect(expandableRow.getAttribute('aria-expanded')).toBe('true')
  })

  it('shows body content when detail data is available', () => {
    mockDetailData = {
      id: 'call-1',
      response_text: 'This is the response',
      reasoning: 'This is the reasoning',
    }
    renderPage()

    // Expand the row
    const expandableRow = screen.getByRole('button', { name: /Expand LLM call/ })
    fireEvent.click(expandableRow)

    // Response should be visible
    expect(screen.getByText('Response')).toBeTruthy()
    expect(screen.getByText('This is the response')).toBeTruthy()

    // Reasoning section should be present but collapsed
    expect(screen.getByText('Reasoning')).toBeTruthy()
  })

  it('collapses expanded row on second click', () => {
    mockDetailData = {
      id: 'call-1',
      response_text: 'Response content',
      reasoning: null,
    }
    renderPage()

    const expandableRow = screen.getByRole('button', { name: /Expand LLM call/ })

    // Expand
    fireEvent.click(expandableRow)
    expect(screen.getByText('Response content')).toBeTruthy()

    // Collapse
    fireEvent.click(expandableRow)
    expect(screen.queryByText('Response content')).toBeNull()
  })

  it('renders "View Full Details" link in expanded row', () => {
    mockDetailData = {
      id: 'call-1',
      response_text: 'Some response',
      reasoning: null,
    }
    renderPage()

    const expandableRow = screen.getByRole('button', { name: /Expand LLM call/ })
    fireEvent.click(expandableRow)

    const detailLink = screen.getByText(/View Full Details/)
    expect(detailLink.closest('a')?.getAttribute('href')).toBe('/llm-calls/call-1')
  })
})
