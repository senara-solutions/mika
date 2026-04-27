import { Link } from 'react-router'
import { useDevRuns, type DevRunsFilters } from '../api/devRuns.ts'
import { Pagination, EmptyState, TaskStatusBadge, ListRow, formatRelativeTime } from '@senara-solutions/ui'
import { useSearchParamsFilter } from '../hooks/useSearchParamsFilter.ts'

const STATUSES = ['', 'pending', 'in_progress', 'blocked', 'completed', 'cancelled', 'failed']

function formatDuration(ms: number | null): string {
  if (ms === null || ms === undefined) return '—'
  const totalSecs = Math.floor(ms / 1000)
  const mins = Math.floor(totalSecs / 60)
  const secs = totalSecs % 60
  if (mins === 0) return `${secs}s`
  return `${mins}m ${secs}s`
}

function formatCost(usd: number | null): string {
  if (usd === null || usd === undefined) return '—'
  return `$${usd.toFixed(2)}`
}

export default function DevRuns() {
  const { searchParams, setSearchParams, updateFilter, setPage } = useSearchParamsFilter()

  const filters: DevRunsFilters = {
    status: searchParams.get('status') ?? undefined,
    page: Number(searchParams.get('page')) || 1,
    per_page: 50,
  }

  const { data, isLoading, error } = useDevRuns(filters)

  return (
    <div>
      <div className="flex items-center justify-between mb-5">
        <div>
          <h2 className="text-heading text-xl font-semibold">Dev Runs</h2>
          <p className="text-sm text-muted/60 mt-1">
            {data
              ? `${data.total} run${data.total !== 1 ? 's' : ''} found`
              : 'Loading dev runs...'}
          </p>
        </div>
      </div>

      {/* Filters */}
      <div className="bg-bg-card border border-white/[0.05] rounded-xl p-3 mb-4">
        <div className="flex flex-wrap items-center gap-3">
          <select
            value={filters.status ?? ''}
            onChange={(e) => updateFilter('status', e.target.value)}
            className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
          >
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s ? s.replace('_', ' ').replace(/\b\w/g, (c) => c.toUpperCase()) : 'All Statuses'}
              </option>
            ))}
          </select>
          {filters.status && (
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
        <EmptyState message="No dev runs match your filters" />
      ) : (
        <>
          <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-white/[0.05] text-muted/60 text-xs uppercase tracking-wider">
                  <th className="text-left px-4 py-3 font-medium">Label</th>
                  <th className="text-left px-4 py-3 font-medium">Source</th>
                  <th className="text-left px-4 py-3 font-medium">Branch</th>
                  <th className="text-left px-4 py-3 font-medium">Status</th>
                  <th className="text-left px-4 py-3 font-medium">PR</th>
                  <th className="text-left px-4 py-3 font-medium">Cost</th>
                  <th className="text-left px-4 py-3 font-medium">Duration</th>
                  <th className="text-left px-4 py-3 font-medium">Created</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {data.data.map((run) => (
                  <ListRow key={run.id} variant="static">
                    <td className="px-4 py-3">
                      <Link
                        to={`/dev-runs/${run.id}`}
                        className="text-accent text-xs hover:text-accent-light transition-colors"
                      >
                        {run.label}
                      </Link>
                      {run.reference_url && (
                        <a
                          href={run.reference_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="ml-2 text-muted/40 text-xs hover:text-muted/60"
                        >
                          issue
                        </a>
                      )}
                    </td>
                    <td className="px-4 py-3">
                      {run.source === 'github_issue' ? (
                        <span className="inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">
                          Issue
                        </span>
                      ) : (
                        <span className="inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-medium bg-purple-500/10 text-purple-400 border border-purple-500/20">
                          Self Dev
                        </span>
                      )}
                    </td>
                    <td className="px-4 py-3 text-xs text-heading font-mono">
                      {run.branch ?? '—'}
                    </td>
                    <td className="px-4 py-3">
                      <TaskStatusBadge status={run.status} />
                    </td>
                    <td className="px-4 py-3">
                      {run.pr_url ? (
                        <a
                          href={run.pr_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-accent text-xs hover:text-accent-light transition-colors"
                        >
                          #{run.pr_number}
                        </a>
                      ) : (
                        <span className="text-muted/30 text-xs">—</span>
                      )}
                    </td>
                    <td className="px-4 py-3 text-xs text-heading">
                      {formatCost(run.cost_usd)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70">
                      {formatDuration(run.duration_ms)}
                    </td>
                    <td className="px-4 py-3 text-xs text-muted/70 font-mono">
                      {formatRelativeTime(run.created_at)}
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
