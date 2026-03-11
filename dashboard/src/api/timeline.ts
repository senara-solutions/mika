import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface TimelineRow {
  trace_id: string | null
  session_id: string | null
  agent_id: string
  event_type: string
  event_subtype: string
  summary: string | null
  created_at: number
}

export interface TimelineFilters {
  agent_id?: string
  event_type?: string
  trace_id?: string
  session_id?: string
  from?: number
  to?: number
  page?: number
  per_page?: number
}

export function useTimeline(filters: TimelineFilters, enabled = true, autoRefresh = true) {
  const isDefaultView =
    !filters.agent_id &&
    !filters.event_type &&
    !filters.trace_id &&
    !filters.session_id &&
    !filters.from &&
    !filters.to &&
    (filters.page ?? 1) === 1

  return useQuery<PaginatedResponse<TimelineRow>>({
    queryKey: ['timeline', filters],
    queryFn: () => apiFetch('/timeline', filters as Record<string, string | number | undefined>),
    refetchInterval: autoRefresh && isDefaultView ? 5000 : false,
    enabled,
  })
}

export function useTraceDetail(traceId: string) {
  return useQuery<TimelineRow[]>({
    queryKey: ['trace', traceId],
    queryFn: () => apiFetch(`/timeline/trace/${traceId}`),
    enabled: !!traceId,
  })
}

export interface TraceMessage {
  id: number
  session_id: string
  agent_id: string
  role: string
  content: string
  channel_type: string
  metadata: string | null
  created_at: number
}

export function useTraceMessages(traceId: string) {
  return useQuery<TraceMessage[]>({
    queryKey: ['trace-messages', traceId],
    queryFn: () => apiFetch(`/timeline/trace/${traceId}/messages`),
    enabled: !!traceId,
  })
}
