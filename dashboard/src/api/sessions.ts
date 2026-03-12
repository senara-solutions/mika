import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'
import type { SessionItem } from './agents.ts'

export interface SessionDetail {
  id: string
  agent_id: string
  channel_type: string
  started_at: number
  ended_at: number | null
  metadata: string | null
}

export interface Message {
  id: number
  session_id: string
  agent_id: string
  role: string
  content: string
  channel_type: string
  metadata: string | null
  created_at: number
}

export interface SessionsFilters {
  agent_id?: string
  channel_type?: string
  session_id?: string
  page?: number
  per_page?: number
}

export function useSessions(filters: SessionsFilters) {
  return useQuery<PaginatedResponse<SessionItem>>({
    queryKey: ['sessions', filters],
    queryFn: () =>
      apiFetch('/sessions', filters as Record<string, string | number | undefined>),
  })
}

export function useSessionDetail(sessionId: string) {
  return useQuery<SessionDetail>({
    queryKey: ['session', sessionId],
    queryFn: () => apiFetch(`/sessions/${sessionId}`),
    enabled: !!sessionId,
  })
}

export function useSessionMessages(sessionId: string, page = 1, perPage = 50) {
  return useQuery<PaginatedResponse<Message>>({
    queryKey: ['session-messages', sessionId, page, perPage],
    queryFn: () =>
      apiFetch(`/sessions/${sessionId}/messages`, { page, per_page: perPage }),
    enabled: !!sessionId,
  })
}
