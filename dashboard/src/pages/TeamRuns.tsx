import { Link } from 'react-router'
import { useTeamRuns, type TeamRunsFilters } from '../api/teams.ts'
import { Pagination, EmptyState, TaskStatusBadge, ListRow, formatRelativeTime } from '@senara-solutions/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'

const STATUSES = ['', 'running', 'completed', 'failed', 'suspended', 'cancelled']

export default function TeamRuns() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()

  const filters: TeamRunsFilters = {
    team_name: searchParams.get('team_name') ?? undefined,
    status: searchParams.get('status') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useTeamRuns(filters)

  return (
    <div>
      <div className="flex items-center justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Team Runs</h2>
          <p className="text-sm text-muted/60 mt-1">
            {data
              ? `${data.total} run${data.total !== 1 ? 's' : ''} found`
              : 'Loading team runs...'}
          </p>
        </div>
      </div>

      {/* Filters */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <input
            type="text"
            placeholder="Filter by team name..."
            value={searchParams.get('team_name') ?? ''}
            onChange={(e) => updateFilter('team_name', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted placeholder:text-muted/30 focus:outline-none focus:border-accent/40 min-w-[180px]"
          />
          <select
            value={filters.status ?? ''}
            onChange={(e) => updateFilter('status', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
          >
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s ? s.charAt(0).toUpperCase() + s.slice(1) : 'All Statuses'}
              </option>
            ))}
          </select>
          {(filters.team_name || filters.status) && (
            <button
              onClick={() => setSearchParams(new URLSearchParams())}
              className="text-xs text-muted/60 hover:text-muted transition-colors"
            >
              Clear
            </button>
          )}
        </div>
      </div>

      {isLoading ? (
        <div className="text-muted/60 py-8 text-center text-sm">Loading...</div>
      ) : error ? (
        <div className="text-red-400 py-8 text-center text-sm">
          Error: {error instanceof Error ? error.message : 'Unknown error'}
        </div>
      ) : !data || data.data.length === 0 ? (
        <EmptyState message="No team runs match your filters" />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Team</th>
                  <th className="text-left px-4 py-3 font-medium">Run ID</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-left px-4 py-3 font-medium">Started</th>
                  <th className="text-left px-4 py-3 font-medium">Iterations</th>
                  <th className="text-left px-4 py-3 font-medium">Trace</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((run) => (
                  <ListRow key={run.id} variant="static">
                    <td className="px-4 py-3 text-xs text-heading font-medium">
                      {run.team_name}
                    </td>
                    <td className="px-4 py-3">
                      <Link
                        to={`/team-runs/${run.id}`}
                        className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                      >
                        {run.id.slice(0, 16)}...
                      </Link>
                    </td>
                    <td className="px-4 py-3">
                      <TaskStatusBadge status={run.status} />
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                      {formatRelativeTime(run.started_at)}
                    </td>
                    <td className="px-4 py-3 text-xs text-heading">
                      {run.iteration}/{run.max_iterations}
                    </td>
                    <td className="px-4 py-3">
                      {run.trace_id && (
                        <Link
                          to={`/traces/${run.trace_id}`}
                          className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                        >
                          {run.trace_id.slice(0, 12)}...
                        </Link>
                      )}
                    </td>
                  </ListRow>
                ))}
              </tbody>
            </table>
          </div>
          <Pagination
            page={filters.page ?? 1}
            perPage={filters.per_page ?? 50}
            total={data.total}
            onPageChange={setPage}
          />
        </>
      )}
    </div>
  )
}
