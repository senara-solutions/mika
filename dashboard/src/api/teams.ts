import { useQuery } from '@tanstack/react-query'
import { apiFetch } from './client.ts'

export interface TeamRun {
  id: string
  team_name: string
  goal: string
  status: string
  failure_reason: string | null
  iteration: number
  max_iterations: number
  deliverable: string | null
  started_at: number
  ended_at: number | null
}

export interface TeamWorkspaceEntry {
  id: number
  run_id: string
  parent_id: number | null
  agent_name: string | null
  entry_type: string
  content: string
  iteration: number
  created_at: number
}

export function useTeamRun(runId: string | undefined) {
  return useQuery<TeamRun>({
    queryKey: ['team-run', runId],
    queryFn: () => apiFetch(`/team-runs/${runId}`),
    enabled: !!runId,
  })
}

export function useTeamWorkspace(runId: string | undefined) {
  return useQuery<TeamWorkspaceEntry[]>({
    queryKey: ['team-workspace', runId],
    queryFn: () => apiFetch(`/team-runs/${runId}/workspace`),
    enabled: !!runId,
  })
}
