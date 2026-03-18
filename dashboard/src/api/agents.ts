import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface Agent {
  id: string
  name: string
  active: boolean
  last_seen: string | null
  created_at: string
  message_count: number
}

export interface CoreMemory {
  key: string
  value: string
  token_count: number
  updated_at: string
}

export interface AgentDetail extends Agent {
  core_memory: CoreMemory[]
  core_memory_edits_this_session: number
  soul_md: string
}

export interface SessionItem {
  id: string
  agent_id: string
  channel_type: string
  started_at: string
  ended_at: string | null
  metadata: string | null
  message_count: number
}

export interface AuditEvent {
  id: number
  session_id: string
  tool_name: string
  target_key: string
  before_value: string | null
  after_value: string
  reasoning: string | null
  created_at: string
}

export function useAgents() {
  return useQuery<Agent[]>({
    queryKey: ['agents'],
    queryFn: () => apiFetch('/agents'),
  })
}

export function useAgentDetail(agentId: string) {
  return useQuery<AgentDetail>({
    queryKey: ['agent', agentId],
    queryFn: () => apiFetch(`/agents/${agentId}`),
    enabled: !!agentId,
  })
}

export function useAgentSessions(agentId: string, page = 1, perPage = 50) {
  return useQuery<PaginatedResponse<SessionItem>>({
    queryKey: ['agent-sessions', agentId, page, perPage],
    queryFn: () =>
      apiFetch(`/agents/${agentId}/sessions`, { page, per_page: perPage }),
    enabled: !!agentId,
  })
}

export function useAgentAudit(agentId: string, page = 1, perPage = 50) {
  return useQuery<PaginatedResponse<AuditEvent>>({
    queryKey: ['agent-audit', agentId, page, perPage],
    queryFn: () =>
      apiFetch(`/agents/${agentId}/audit`, { page, per_page: perPage }),
    enabled: !!agentId,
  })
}
