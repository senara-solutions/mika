import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import Home from '../Home.tsx'

// Mock recharts to avoid jsdom rendering issues
vi.mock('recharts', () => ({
  AreaChart: ({ children }: { children: React.ReactNode }) => <svg data-testid="area-chart">{children}</svg>,
  Area: ({ dataKey }: { dataKey: string }) => <div data-testid={`area-${dataKey}`} />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Legend: () => <div data-testid="legend" />,
  ResponsiveContainer: ({ children }: { children: React.ReactNode }) => <div data-testid="responsive-container">{children}</div>,
}))

const mockAgents = [
  { id: 'mika-dev', name: 'mika-dev', active: true, last_seen: '2026-05-06T10:00:00Z', created_at: '2026-01-01T00:00:00Z', message_count: 100 },
  { id: 'mika-qa', name: 'mika-qa', active: false, last_seen: null, created_at: '2026-01-02T00:00:00Z', message_count: 50 },
]

let useAgentsReturn = {
  data: mockAgents as typeof mockAgents | undefined,
  isLoading: false,
  error: null as Error | null,
  refetch: vi.fn(),
}

// Mock useAgents (used by Home page gate AND AgentsSummaryWidget)
vi.mock('../../api/agents.ts', () => ({
  useAgents: () => useAgentsReturn,
}))

// Mock the API client used by inline useQuery calls in widgets
vi.mock('../../api/client.ts', () => ({
  apiFetch: vi.fn(() => Promise.resolve({ data: [], total: 0, page: 1, per_page: 5 })),
}))

function renderHome() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/']}>
        <Home />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('Home page', () => {
  beforeEach(() => {
    useAgentsReturn = {
      data: mockAgents,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    }
  })

  it('renders the overview header with LIVE badge', () => {
    renderHome()
    expect(screen.getByText('Overview')).toBeTruthy()
    expect(screen.getByText('Live')).toBeTruthy()
  })

  it('renders all widget section labels', () => {
    renderHome()
    // CSS uppercase transforms "Agents" → "AGENTS" visually; DOM text is lowercase
    expect(screen.getByText('Agents')).toBeTruthy()
    expect(screen.getByText('Work Items')).toBeTruthy()
    expect(screen.getByText('Dev Runs')).toBeTruthy()
    expect(screen.getByText('Team Runs')).toBeTruthy()
    expect(screen.getByText('Cost (24h)')).toBeTruthy()
    expect(screen.getByText('Recent Activity')).toBeTruthy()
  })

  it('renders agent data in the agents widget', () => {
    renderHome()
    expect(screen.getByText('mika-dev')).toBeTruthy()
    expect(screen.getByText('mika-qa')).toBeTruthy()
  })

  it('shows active status badge for active agents', () => {
    renderHome()
    expect(screen.getByText('Active')).toBeTruthy()
    expect(screen.getByText('Inactive')).toBeTruthy()
  })

  it('shows "Never seen" for agent with null last_seen', () => {
    renderHome()
    expect(screen.getByText('Never seen')).toBeTruthy()
  })

  it('renders "View all" links for each section', () => {
    renderHome()
    const viewAllLinks = screen.getAllByText('View all')
    expect(viewAllLinks.length).toBeGreaterThanOrEqual(6)
  })

  it('navigates to agent detail page via link', () => {
    renderHome()
    const agentLink = screen.getByRole('link', { name: /mika-dev/i })
    expect(agentLink.getAttribute('href')).toBe('/agents/mika-dev')
  })
})

describe('Home page — fresh install', () => {
  it('shows welcome empty state when no agents', () => {
    useAgentsReturn = {
      data: [],
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    }
    renderHome()
    expect(screen.getByText('Welcome to Mika')).toBeTruthy()
  })

  it('shows loading state while agents query resolves', () => {
    useAgentsReturn = {
      data: undefined,
      isLoading: true,
      error: null,
      refetch: vi.fn(),
    }
    renderHome()
    // LoadingState renders aria-label
    expect(screen.getByRole('status')).toBeTruthy()
  })

  it('shows error state when agents query fails', () => {
    useAgentsReturn = {
      data: undefined,
      isLoading: false,
      error: new Error('Network error'),
      refetch: vi.fn(),
    }
    renderHome()
    // ErrorState renders retry button
    expect(screen.getByRole('button', { name: /retry/i })).toBeTruthy()
  })
})
