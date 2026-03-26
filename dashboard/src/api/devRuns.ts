import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, TOKEN, API_BASE, type PaginatedResponse } from './client.ts'

export interface DevRun {
  id: string
  agent_id: string
  source: string | null
  label: string
  status: string
  reference_url: string | null
  created_at: string
  updated_at: string
  completed_at: string | null
  // Extracted from metadata.claude_pilot:
  branch: string | null
  repo: string | null
  pr_number: number | null
  pr_url: string | null
  cost_usd: number | null
  duration_ms: number | null
  turns: number | null
  session_id: string | null
}

export interface DevRunsFilters {
  status?: string
  page?: number
  per_page?: number
}

export function useDevRuns(filters: DevRunsFilters) {
  return useQuery<PaginatedResponse<DevRun>>({
    queryKey: ['dev-runs', filters],
    queryFn: () =>
      apiFetch('/dev-runs', filters as Record<string, string | number | undefined>),
  })
}

export function useDevRun(taskId: string | undefined) {
  return useQuery<DevRun>({
    queryKey: ['dev-run', taskId],
    queryFn: () => apiFetch(`/dev-runs/${taskId}`),
    enabled: !!taskId,
  })
}

export function useMergeDevRun() {
  const queryClient = useQueryClient()
  return useMutation<{ merged: boolean; pr_url: string }, Error, string>({
    mutationFn: async (taskId: string) => {
      const url = new URL(`${API_BASE}/dev-runs/${taskId}/merge`, window.location.origin)
      const res = await fetch(url.toString(), {
        method: 'POST',
        headers: {
          ...(TOKEN ? { Authorization: `Bearer ${TOKEN}` } : {}),
        },
      })
      if (!res.ok) {
        const body = await res.json().catch(() => ({ error: res.statusText }))
        throw new Error(body.error ?? `HTTP ${res.status}`)
      }
      return res.json()
    },
    onSuccess: (_data, taskId) => {
      queryClient.invalidateQueries({ queryKey: ['dev-run', taskId] })
      queryClient.invalidateQueries({ queryKey: ['dev-runs'] })
    },
  })
}
