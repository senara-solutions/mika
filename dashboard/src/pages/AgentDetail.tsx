import { useState } from 'react'
import { useParams, Link, useSearchParams } from 'react-router'
import { useAgentDetail, useAgentSessions, useAgentAudit, useAgentFacts, type CoreMemory, type FactEntry, type AuditEvent } from '../api/agents.ts'
import { StatusBadge, Pagination, EmptyState, LoadingState, ErrorState, formatApiError, MarkdownContent, formatRelativeTime, CopyButton, TokenBudgetBar } from '@senara-solutions/ui'
import { ArrowLeft, User, Brain, Target, Users, GitBranch, ChevronDown } from 'lucide-react'

const BLOCK_TOKEN_CAP = 500
const EDIT_BUDGET = 3

const BLOCK_ICONS: Record<string, typeof User> = {
  user_summary: User,
  self_model: Brain,
  current_priorities: Target,
  key_people: Users,
  workflows: GitBranch,
}

/**
 * Attempts to parse a value as JSON and render as a definition list.
 * Falls back to MarkdownContent for plain text / markdown content.
 */
function ContentRenderer({ value, expanded }: { value: string; expanded: boolean }) {
  if (!expanded) {
    return (
      <p className="text-xs text-muted/70 font-mono leading-relaxed line-clamp-3 min-h-[3lh]">
        {value || '\u00A0'}
      </p>
    )
  }

  // Try to parse as JSON for structured rendering
  let parsed: Record<string, unknown> | null = null
  try {
    const result = JSON.parse(value)
    if (typeof result === 'object' && result !== null && !Array.isArray(result)) {
      parsed = result as Record<string, unknown>
    }
  } catch {
    // Not JSON — render as markdown
  }

  if (parsed) {
    return (
      <dl className="space-y-2 text-xs">
        {Object.entries(parsed).map(([key, val]) => (
          <div key={key} className="border-b border-white/[0.03] pb-2 last:border-0 last:pb-0">
            <dt className="text-muted/60 font-semibold uppercase tracking-wider text-[10px] mb-0.5">
              {key}
            </dt>
            <dd className="text-muted/80 font-mono leading-relaxed whitespace-pre-wrap">
              {typeof val === 'object' ? JSON.stringify(val, null, 2) : String(val)}
            </dd>
          </div>
        ))}
      </dl>
    )
  }

  return (
    <div className="text-xs text-muted/70 leading-relaxed">
      <MarkdownContent content={value || '\u00A0'} />
    </div>
  )
}

function MemoryBlock({ mem }: { mem: CoreMemory }) {
  const [expanded, setExpanded] = useState(false)
  const Icon = BLOCK_ICONS[mem.key] ?? Brain
  const isLong = mem.value.length > 120

  return (
    <div className="bg-bg rounded-lg p-3 border border-white/[0.04]">
      <div className="flex items-center justify-between mb-1.5">
        <div className="flex items-center gap-1.5">
          <Icon size={12} className="text-accent shrink-0" />
          <span className="text-[11px] text-heading font-semibold uppercase tracking-wider truncate">
            {mem.key}
          </span>
        </div>
        <div className="flex items-center gap-1">
          {expanded && <CopyButton text={mem.value} className="opacity-60 hover:opacity-100" />}
          {isLong && (
            <button
              type="button"
              onClick={() => setExpanded(!expanded)}
              className="p-0.5 rounded hover:bg-white/[0.05] transition-colors"
              aria-expanded={expanded}
              aria-label={expanded ? 'Collapse section' : 'Expand section'}
            >
              <ChevronDown
                size={12}
                className={`text-muted/40 transition-transform duration-200 ${expanded ? 'rotate-180' : ''}`}
              />
            </button>
          )}
        </div>
      </div>

      <div className={expanded ? 'max-h-96 overflow-y-auto' : ''}>
        <ContentRenderer value={mem.value} expanded={expanded} />
      </div>

      <div className="mt-2">
        <TokenBudgetBar value={mem.token_count} max={BLOCK_TOKEN_CAP} label="Tokens" />
      </div>

      <div className="flex items-center justify-between mt-1.5">
        <span className="text-[9px] text-muted/30 font-mono">
          Updated {formatRelativeTime(mem.updated_at)}
        </span>
      </div>
    </div>
  )
}

function FactsList({
  facts,
  isLoading: loading,
  total,
  page,
  onPageChange,
}: {
  facts: FactEntry[] | undefined
  isLoading: boolean
  total: number
  page: number
  onPageChange: (p: number) => void
}) {
  if (loading) {
    return <LoadingState variant="detail" />
  }
  if (!facts || facts.length === 0) {
    return <EmptyState message="No facts stored" />
  }

  // Group by category
  const grouped = facts.reduce<Record<string, FactEntry[]>>((acc, fact) => {
    const cat = fact.category
    if (!acc[cat]) acc[cat] = []
    acc[cat].push(fact)
    return acc
  }, {})

  return (
    <div>
      <div className="space-y-3 max-h-[32rem] overflow-y-auto">
        {Object.entries(grouped).map(([category, items]) => (
          <div key={category}>
            <h4 className="text-[10px] text-muted/50 font-semibold uppercase tracking-wider mb-1.5">
              {category} ({items.length})
            </h4>
            <div className="space-y-1">
              {items.map((fact) => (
                <div
                  key={`${fact.category}-${fact.id}`}
                  className="bg-bg rounded-lg px-3 py-2 border border-white/[0.04]"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-heading font-medium truncate">
                      {fact.key}
                    </span>
                    <span className="text-[9px] text-muted/30 font-mono shrink-0 ml-2">
                      {formatRelativeTime(fact.updated_at)}
                    </span>
                  </div>
                  <p className="text-[11px] text-muted/60 font-mono mt-0.5 line-clamp-2">
                    {fact.value}
                  </p>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
      {total > 20 && (
        <Pagination
          page={page}
          perPage={20}
          total={total}
          onPageChange={onPageChange}
        />
      )}
    </div>
  )
}

function HistoryList({
  events,
  isLoading: loading,
  total,
  page,
  onPageChange,
}: {
  events: AuditEvent[] | undefined
  isLoading: boolean
  total: number
  page: number
  onPageChange: (p: number) => void
}) {
  if (loading) {
    return <LoadingState variant="detail" />
  }
  if (!events || events.length === 0) {
    return <EmptyState message="No core memory edits recorded" />
  }

  return (
    <div>
      <div className="space-y-2 max-h-[32rem] overflow-y-auto">
        {events.map((e) => (
          <div
            key={e.id}
            className="bg-bg rounded-lg px-3 py-2 border border-white/[0.04]"
          >
            <div className="flex items-center justify-between mb-1">
              <span className="text-[11px] text-heading font-semibold uppercase tracking-wider">
                {e.target_key}
              </span>
              <span className="text-[9px] text-muted/30 font-mono">
                {formatRelativeTime(e.created_at)}
              </span>
            </div>
            {e.after_value && (
              <div className="text-xs mt-0.5">
                {e.before_value && (
                  <p className="text-muted/40 line-through truncate mb-0.5">
                    {e.before_value.slice(0, 80)}
                  </p>
                )}
                <p className="text-muted/60 truncate">
                  {e.after_value.slice(0, 120)}
                </p>
              </div>
            )}
            {e.reasoning && (
              <p className="text-[10px] text-muted/30 mt-1 italic truncate">
                {e.reasoning.slice(0, 100)}
              </p>
            )}
          </div>
        ))}
      </div>
      {total > 10 && (
        <Pagination
          page={page}
          perPage={10}
          total={total}
          onPageChange={onPageChange}
        />
      )}
    </div>
  )
}

type MemoryTab = 'sections' | 'facts' | 'history'

const TAB_LABELS: { key: MemoryTab; label: string }[] = [
  { key: 'sections', label: 'Sections' },
  { key: 'facts', label: 'Facts' },
  { key: 'history', label: 'History' },
]

const VALID_MEMORY_TABS: readonly MemoryTab[] = TAB_LABELS.map((t) => t.key)

function isMemoryTab(value: string | null): value is MemoryTab {
  return VALID_MEMORY_TABS.includes(value as MemoryTab)
}

export default function AgentDetail() {
  const { agentId } = useParams<{ agentId: string }>()
  const [searchParams, setSearchParams] = useSearchParams()
  const [sessionsPage, setSessionsPage] = useState(1)
  const [factsPage, setFactsPage] = useState(1)
  const [historyPage, setHistoryPage] = useState(1)

  const rawTab = searchParams.get('tab')
  const memoryTab: MemoryTab = isMemoryTab(rawTab) ? rawTab : 'sections'

  const handleTabChange = (tab: MemoryTab) => {
    const next = new URLSearchParams(searchParams)
    if (tab === 'sections') {
      next.delete('tab')
    } else {
      next.set('tab', tab)
    }
    setSearchParams(next)
    if (tab === 'facts') setFactsPage(1)
    if (tab === 'history') setHistoryPage(1)
  }

  const { data: agent, isLoading, error, refetch } = useAgentDetail(agentId ?? '')
  const { data: sessions } = useAgentSessions(agentId ?? '', sessionsPage)
  const { data: audit } = useAgentAudit(agentId ?? '', 1)
  const { data: facts, isLoading: factsLoading } = useAgentFacts(
    agentId ?? '',
    factsPage,
    20,
    memoryTab === 'facts',
  )
  const { data: history, isLoading: historyLoading } = useAgentAudit(
    agentId ?? '',
    historyPage,
    10,
    { tool_name: 'update_core_memory' },
    memoryTab === 'history',
  )

  if (isLoading) {
    return <LoadingState variant="detail" />
  }
  if (error) {
    return <ErrorState message={formatApiError(error)} retry={() => refetch()} />
  }
  if (!agent) {
    return <EmptyState message="Agent not found" />
  }

  const editsUsed = agent.core_memory_edits_this_session

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Link
            to="/agents"
            className="p-1.5 rounded-lg hover:bg-white/[0.05] text-muted transition-colors"
          >
            <ArrowLeft size={18} />
          </Link>
          <div className="w-10 h-10 rounded-xl bg-accent/10 flex items-center justify-center">
            <span className="text-accent font-semibold">
              {agent.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div>
            <div className="flex items-center gap-3">
              <h2 className="text-heading text-xl font-semibold">{agent.name}</h2>
              <StatusBadge variant={agent.active ? 'success' : 'neutral'} label={agent.active ? 'Active' : 'Inactive'} />
            </div>
            <p className="text-xs text-muted/50 font-mono mt-0.5">
              ID: {agent.id} &middot; Created {formatRelativeTime(agent.created_at)}
            </p>
          </div>
        </div>
      </div>

      {/* Two-column layout: Core Memory + Soul.md */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 mb-6">
        {/* Core Memory / Facts / History */}
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
          <div className="flex items-center justify-between mb-4">
            {/* Tab switcher */}
            <div className="flex items-center gap-1">
              {TAB_LABELS.map((tab) => (
                <button
                  key={tab.key}
                  type="button"
                  onClick={() => handleTabChange(tab.key)}
                  className={`text-sm font-semibold px-2 py-0.5 rounded transition-colors ${
                    memoryTab === tab.key
                      ? 'text-heading bg-white/[0.05]'
                      : 'text-muted/50 hover:text-muted'
                  }`}
                >
                  {tab.label}
                  {tab.key === 'facts' && facts && facts.total > 0 ? ` (${facts.total})` : ''}
                  {tab.key === 'history' && history && history.total > 0 ? ` (${history.total})` : ''}
                </button>
              ))}
            </div>
            {memoryTab === 'sections' && (
              <div
                className="flex items-center gap-1.5"
                title={`Core memory can be updated ${EDIT_BUDGET} times per conversation session via the update_core_memory tool. ${editsUsed} of ${EDIT_BUDGET} used.`}
              >
                <StatusBadge
                  variant={editsUsed >= EDIT_BUDGET ? 'error' : editsUsed >= EDIT_BUDGET - 1 ? 'warning' : 'success'}
                  label={editsUsed >= EDIT_BUDGET ? 'Edit budget used' : `${EDIT_BUDGET - editsUsed} edits remaining`}
                />
              </div>
            )}
          </div>

          {memoryTab === 'sections' && (
            agent.core_memory.length === 0 ? (
              <EmptyState message="No core memory blocks" />
            ) : (
              <>
                <div className="grid grid-cols-2 gap-2">
                  {agent.core_memory
                    .filter((m) => m.key !== 'workflows')
                    .map((mem) => (
                      <MemoryBlock key={mem.key} mem={mem} />
                    ))}
                </div>
                {agent.core_memory
                  .filter((m) => m.key === 'workflows')
                  .map((mem) => (
                    <div key={mem.key} className="mt-2">
                      <MemoryBlock mem={mem} />
                    </div>
                  ))}
              </>
            )
          )}

          {memoryTab === 'facts' && (
            <FactsList
              facts={facts?.data}
              isLoading={factsLoading}
              total={facts?.total ?? 0}
              page={factsPage}
              onPageChange={setFactsPage}
            />
          )}

          {memoryTab === 'history' && (
            <HistoryList
              events={history?.data}
              isLoading={historyLoading}
              total={history?.total ?? 0}
              page={historyPage}
              onPageChange={setHistoryPage}
            />
          )}
        </div>

        {/* Soul.md */}
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
          <h3 className="text-heading text-sm font-semibold mb-4">soul.md</h3>
          {agent.soul_md ? (
            <div className="bg-bg rounded-xl p-4 border border-white/[0.04] max-h-96 overflow-y-auto">
              <MarkdownContent content={agent.soul_md} />
            </div>
          ) : (
            <EmptyState message="No soul.md defined" />
          )}
        </div>
      </div>

      {/* Recent Audit Events */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mb-4">
        <h3 className="text-heading text-sm font-semibold mb-4">Recent Audit Events</h3>
        {!audit || audit.data.length === 0 ? (
          <EmptyState message="No audit events for this agent" />
        ) : (
          <div className="space-y-3">
            {audit.data.slice(0, 5).map((e) => (
              <div
                key={e.id}
                className="flex items-start gap-3 py-2 border-b border-white/[0.03] last:border-0"
              >
                <span className="text-[10px] text-muted/50 font-mono whitespace-nowrap pt-0.5">
                  {e.created_at}
                </span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-amber-400 font-mono font-medium uppercase">
                      {e.tool_name}
                    </span>
                    <span className="text-xs text-muted/40">{e.target_key}</span>
                  </div>
                  {e.after_value && (
                    <p className="text-xs text-muted/60 mt-0.5 truncate">
                      {e.before_value && (
                        <span className="text-red-400/60 line-through mr-1">{e.before_value.slice(0, 40)}</span>
                      )}
                      <span className="text-emerald-400/60">{e.after_value.slice(0, 60)}</span>
                    </p>
                  )}
                </div>
                <span className="text-[10px] text-muted/30 font-mono shrink-0">
                  {e.session_id?.slice(0, 8)}
                </span>
              </div>
            ))}
            {audit.total > 5 && (
              <div className="text-center pt-2">
                <Link
                  to={`/?event_type=audit&agent_id=${agentId}`}
                  className="text-xs text-accent hover:text-accent-light transition-colors"
                >
                  View Full Logs ({audit.total} total)
                </Link>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Recent Sessions */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-heading text-sm font-semibold">Recent Sessions</h3>
          {sessions && sessions.total > 0 && (
            <span className="text-xs text-muted/50">{sessions.total} total</span>
          )}
        </div>
        {!sessions || sessions.data.length === 0 ? (
          <EmptyState message="No sessions for this agent" />
        ) : (
          <>
            <div className="overflow-hidden rounded-xl border border-white/[0.04]">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-white/[0.04] text-muted/50 text-xs uppercase tracking-wider bg-white/[0.02]">
                    <th className="text-left px-4 py-2.5 font-medium">Session ID</th>
                    <th className="text-left px-4 py-2.5 font-medium">Duration</th>
                    <th className="text-left px-4 py-2.5 font-medium">Messages</th>
                    <th className="text-left px-4 py-2.5 font-medium">Channel</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/[0.03]">
                  {sessions.data.map((s) => (
                    <tr key={s.id} className="hover:bg-white/[0.02] transition-colors">
                      <td className="px-4 py-2.5">
                        <Link
                          to={`/sessions/${s.id}`}
                          className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
                        >
                          {s.id}
                        </Link>
                      </td>
                      <td className="px-4 py-2.5 text-xs text-muted font-mono">
                        {s.ended_at
                          ? `${Math.round((new Date(s.ended_at).getTime() - new Date(s.started_at).getTime()) / 60000)}m`
                          : 'ongoing'}
                      </td>
                      <td className="px-4 py-2.5 text-xs text-heading font-medium">
                        {s.message_count}
                      </td>
                      <td className="px-4 py-2.5">
                        <span className="text-xs text-muted bg-white/[0.04] px-1.5 py-0.5 rounded">
                          {s.channel_type}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <Pagination
              page={sessionsPage}
              perPage={50}
              total={sessions.total}
              onPageChange={setSessionsPage}
            />
          </>
        )}
      </div>
    </div>
  )
}
