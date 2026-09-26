import { useQuery, keepPreviousData } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface TeamRun {
  id: string
  team_name: string
  goal: string
  status: string
  failure_reason: string | null
  iteration: number
  max_iterations: number
  deliverable: string | null
  started_at: string
  ended_at: string | null
  trace_id: string | null
}

export interface TeamWorkspaceEntry {
  id: number
  run_id: string
  parent_id: number | null
  agent_name: string | null
  entry_type: string
  content: string
  iteration: number
  created_at: string
}

export interface AgentResultSummary {
  agent_name: string
  response_preview: string
}

export interface TaskStatusSummary {
  agent_id: string
  label: string
  status: string
  task_id: string
}

export interface TeamRunSummary {
  run: TeamRun
  agent_results: AgentResultSummary[]
  task_statuses: TaskStatusSummary[]
  pending_tasks: TaskStatusSummary[]
  critic_feedback: string | null
}

export interface TeamRunsFilters {
  team_name?: string
  status?: string
  from?: string
  to?: string
  page?: number
  per_page?: number
}

export function useTeamRuns(filters: TeamRunsFilters) {
  return useQuery<PaginatedResponse<TeamRun>>({
    queryKey: ['team-runs', filters],
    queryFn: () =>
      apiFetch('/team-runs', filters as Record<string, string | number | undefined>),
  })
}

export function useTeamRun(runId: string | undefined, refetchInterval?: number | false) {
  return useQuery<TeamRun>({
    queryKey: ['team-run', runId],
    queryFn: () => apiFetch(`/team-runs/${runId}`),
    enabled: !!runId,
    refetchInterval,
    placeholderData: keepPreviousData,
  })
}

export function useTeamRunSummary(runId: string | undefined, refetchInterval?: number | false) {
  return useQuery<TeamRunSummary>({
    queryKey: ['team-run-summary', runId],
    queryFn: () => apiFetch(`/team-runs/${runId}/summary`),
    enabled: !!runId,
    refetchInterval,
    placeholderData: keepPreviousData,
  })
}

export function useTeamWorkspace(runId: string | undefined, refetchInterval?: number | false) {
  return useQuery<TeamWorkspaceEntry[]>({
    queryKey: ['team-workspace', runId],
    queryFn: () => apiFetch(`/team-runs/${runId}/workspace`),
    enabled: !!runId,
    refetchInterval,
    placeholderData: keepPreviousData,
  })
}
