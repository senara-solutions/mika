import { Link } from 'react-router'
import { useAgents } from '../../api/agents.ts'
import { StatusBadge, LoadingState, ErrorState, EmptyState, formatApiError, formatRelativeTime } from '@senara-solutions/ui'
import WidgetSection from './WidgetSection.tsx'

/**
 * Agents summary widget for the landing page.
 * Note: useAgents() returns unpaginated Agent[] and doesn't accept refetchInterval.
 * The landing page's agents gate in Home.tsx handles the polling lifecycle.
 */
export default function AgentsSummaryWidget() {
  const { data: agents, isLoading, error, refetch } = useAgents()

  return (
    <WidgetSection
      label="Agents"
      count={agents?.length}
      viewAllTo="/agents"
    >
      {isLoading ? (
        <LoadingState variant="list" rows={3} />
      ) : error ? (
        <ErrorState message={formatApiError(error)} retry={() => refetch()} />
      ) : !agents || agents.length === 0 ? (
        <EmptyState message="No agents registered" />
      ) : (
        <div className="space-y-1">
          {agents.map((agent) => (
            <Link
              key={agent.id}
              to={`/agents/${agent.id}`}
              className="flex items-center justify-between px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors group"
            >
              <div className="flex items-center gap-3 min-w-0">
                <div className="w-8 h-8 rounded-lg bg-accent/10 flex items-center justify-center shrink-0">
                  <span className="text-accent font-semibold text-xs">
                    {agent.name.charAt(0).toUpperCase()}
                  </span>
                </div>
                <div className="min-w-0">
                  <p className="text-sm text-heading font-medium truncate group-hover:text-accent-light transition-colors">
                    {agent.name}
                  </p>
                  <p className="text-[10px] text-muted/40">
                    {agent.last_seen ? `Last seen ${formatRelativeTime(agent.last_seen)}` : 'Never seen'}
                  </p>
                </div>
              </div>
              <StatusBadge
                variant={agent.active ? 'success' : 'neutral'}
                label={agent.active ? 'Active' : 'Inactive'}
                dotPulse={agent.active}
              />
            </Link>
          ))}
        </div>
      )}
    </WidgetSection>
  )
}
