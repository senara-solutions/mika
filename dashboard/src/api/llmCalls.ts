import { useQuery, keepPreviousData } from '@tanstack/react-query'
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
  prompt_variant: string | null
  created_at: string
  response_text: string | null
  reasoning: string | null
  /** Estimated cost in USD, computed from token counts and provider pricing. */
  cost_usd: number | null
}

export interface LlmCallsFilters {
  agent_id?: string
  trace_id?: string
  model?: string
  from?: string
  to?: string
  page?: number
  per_page?: number
}

export function useLlmCalls(filters: LlmCallsFilters, refetchInterval?: number | false) {
  return useQuery<PaginatedResponse<LlmCallRow>>({
    queryKey: ['llm-calls', filters],
    queryFn: () =>
      apiFetch('/llm-calls', filters as Record<string, string | number | undefined>),
    refetchInterval,
    placeholderData: keepPreviousData,
  })
}

export function useLlmCall(id: string | undefined) {
  return useQuery<LlmCallRow>({
    queryKey: ['llm-call', id],
    queryFn: () => apiFetch(`/llm-calls/${id}`),
    enabled: !!id,
  })
}

export function useTraceLlmCalls(traceId: string, refetchInterval?: number | false) {
  return useQuery<LlmCallRow[]>({
    queryKey: ['trace-llm-calls', traceId],
    queryFn: () => apiFetch(`/traces/${traceId}/llm-calls`),
    enabled: !!traceId,
    refetchInterval,
    placeholderData: keepPreviousData,
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

export function useLlmCallToolCalls(llmCallId: string | undefined) {
  return useQuery<import('./toolCalls.ts').ToolCallRow[]>({
    queryKey: ['llm-call-tool-calls', llmCallId],
    queryFn: () => apiFetch(`/llm-calls/${llmCallId}/tool-calls`),
    enabled: !!llmCallId,
  })
}

// Cost trend types and hook

export interface CostTrendBucket {
  timestamp: string
  cost_usd: number
  input_tokens: number
  output_tokens: number
  call_count: number
  agent_id: string | null
}

export interface CostTrendResponse {
  buckets: CostTrendBucket[]
  bucket_size: string
  has_estimated_pricing: boolean
  estimated_models: string[]
}

export interface CostTrendFilters {
  agent_id?: string
  model?: string
  from?: string
  to?: string
  bucket?: string
}

export function useCostTrend(filters: CostTrendFilters, refetchInterval?: number | false) {
  return useQuery<CostTrendResponse>({
    queryKey: ['cost-trend', filters],
    queryFn: () =>
      apiFetch('/llm-calls/cost-trend', filters as Record<string, string | undefined>),
    refetchInterval,
    placeholderData: keepPreviousData,
  })
}
