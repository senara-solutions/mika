import { useAgents } from '../api/agents.ts'
import { StatusBadge, LoadingState, ErrorState, EmptyState, formatApiError } from '@senara-solutions/ui'
import { Bot } from 'lucide-react'
import AgentsSummaryWidget from '../components/home/AgentsSummaryWidget.tsx'
import WorkItemsWidget from '../components/home/WorkItemsWidget.tsx'
import DevRunsWidget from '../components/home/DevRunsWidget.tsx'
import TeamRunsWidget from '../components/home/TeamRunsWidget.tsx'
import CostSummaryWidget from '../components/home/CostSummaryWidget.tsx'
import RecentActivityWidget from '../components/home/RecentActivityWidget.tsx'

/** Shared auto-refresh interval for all landing page widgets (ms). */
const HOME_REFETCH_INTERVAL = 15_000

export default function Home() {
  // Gate on agents query — fresh installs show a cohesive empty state
  const { data: agents, isLoading: agentsLoading, error: agentsError, refetch: agentsRefetch } = useAgents()

  return (
    <div>
      {/* Header */}
      <div className="flex items-start justify-between mb-6">
        <div>
          <div className="flex items-center gap-3">
            <h2 className="text-heading text-xl font-semibold">Overview</h2>
            <StatusBadge variant="success" label="Live" dotPulse />
          </div>
          <p className="text-sm text-muted/60 mt-1">
            Current state across all agents
          </p>
        </div>
      </div>

      {/* Page-level gate: loading / error / fresh-install empty */}
      {agentsLoading ? (
        <LoadingState variant="list" rows={6} />
      ) : agentsError ? (
        <ErrorState message={formatApiError(agentsError)} retry={() => agentsRefetch()} />
      ) : !agents || agents.length === 0 ? (
        <div className="flex items-center justify-center py-20">
          <EmptyState
            title="Welcome to Mika"
            message="No agents are provisioned yet. Start by creating an agent to see your overview."
            icon={<Bot size={32} />}
          />
        </div>
      ) : (
        <div className="space-y-4">
          {/* Agents summary */}
          <AgentsSummaryWidget />

          {/* Active work — full-width stacked for scanning density */}
          <WorkItemsWidget refetchInterval={HOME_REFETCH_INTERVAL} />
          <DevRunsWidget refetchInterval={HOME_REFETCH_INTERVAL} />
          <TeamRunsWidget refetchInterval={HOME_REFETCH_INTERVAL} />

          {/* Secondary signals — side by side on wide viewports */}
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
            <CostSummaryWidget refetchInterval={HOME_REFETCH_INTERVAL} />
            <RecentActivityWidget refetchInterval={HOME_REFETCH_INTERVAL} />
          </div>
        </div>
      )}
    </div>
  )
}
