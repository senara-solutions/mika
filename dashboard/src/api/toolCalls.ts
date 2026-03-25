import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface ToolCallRow {
  id: string
  agent_id: string
  session_id: string
  trace_id: string
  llm_call_id: string
  step: number
  tool_name: string
  tool_source: string
  skill_name: string | null
  input: string | null
  output: string | null
  success: boolean
  non_zero_exit: boolean
  latency_ms: number
  error_message: string | null
  created_at: string
}

export interface ToolCallsFilters {
  agent_id?: string
  trace_id?: string
  tool_name?: string
  success?: string
  from?: number
  to?: number
  page?: number
  per_page?: number
}

export interface SkillSummary {
  loaded_skills: string[]
  skill_count: number
}

export function useToolCalls(filters: ToolCallsFilters) {
  return useQuery<PaginatedResponse<ToolCallRow>>({
    queryKey: ['tool-calls', filters],
    queryFn: () =>
      apiFetch('/tool-calls', filters as Record<string, string | number | undefined>),
  })
}

export function useTraceToolCalls(traceId: string) {
  return useQuery<ToolCallRow[]>({
    queryKey: ['trace-tool-calls', traceId],
    queryFn: () => apiFetch(`/traces/${traceId}/tool-calls`),
    enabled: !!traceId,
  })
}

export function useSessionToolCalls(sessionId: string, page = 1, perPage = 50) {
  return useQuery<PaginatedResponse<ToolCallRow>>({
    queryKey: ['session-tool-calls', sessionId, page, perPage],
    queryFn: () =>
      apiFetch(`/sessions/${sessionId}/tool-calls`, { page, per_page: perPage }),
    enabled: !!sessionId,
  })
}

export function useSessionSkills(sessionId: string) {
  return useQuery<SkillSummary>({
    queryKey: ['session-skills', sessionId],
    queryFn: () => apiFetch(`/sessions/${sessionId}/skills`),
    enabled: !!sessionId,
  })
}
