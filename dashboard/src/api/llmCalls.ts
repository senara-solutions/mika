import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface LlmCallRow {
  id: string
  agent_id: string
  session_id: string
  trace_id: string
  provider: string
  model: string
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number | null
  cache_write_tokens: number | null
  latency_ms: number
  stop_reason: string | null
  status: string
  error_message: string | null
  step: number
  created_at: string
}

export interface LlmCallsFilters {
  agent_id?: string
  trace_id?: string
  model?: string
  from?: number
  to?: number
  page?: number
  per_page?: number
}

export function useLlmCalls(filters: LlmCallsFilters) {
  return useQuery<PaginatedResponse<LlmCallRow>>({
    queryKey: ['llm-calls', filters],
    queryFn: () =>
      apiFetch('/llm-calls', filters as Record<string, string | number | undefined>),
  })
}

export function useLlmCall(id: string | undefined) {
  return useQuery<LlmCallRow>({
    queryKey: ['llm-call', id],
    queryFn: () => apiFetch(`/llm-calls/${id}`),
    enabled: !!id,
  })
}

export function useTraceLlmCalls(traceId: string) {
  return useQuery<LlmCallRow[]>({
    queryKey: ['trace-llm-calls', traceId],
    queryFn: () => apiFetch(`/traces/${traceId}/llm-calls`),
    enabled: !!traceId,
  })
}

export function useSessionLlmCalls(sessionId: string, page = 1, perPage = 50) {
  return useQuery<PaginatedResponse<LlmCallRow>>({
    queryKey: ['session-llm-calls', sessionId, page, perPage],
    queryFn: () =>
      apiFetch(`/sessions/${sessionId}/llm-calls`, { page, per_page: perPage }),
    enabled: !!sessionId,
  })
}
