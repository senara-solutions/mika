import { useQuery } from '@tanstack/react-query'
import { apiFetch, type PaginatedResponse } from './client.ts'

export interface TaskItem {
  id: string
  agent_id: string
  label: string
  trigger_type: string
  action_type: string
  status: string
  team_run_id: string | null
  parent_task_id: string | null
  depth: number
  source: string | null
  reference_url: string | null
  cron_expr: string | null
  next_fire_at: string | null
  fired_at: string | null
  completed_at: string | null
  created_by_session: string | null
  created_trace_id: string | null
  execution_trace_id: string | null
  created_at: string
  updated_at: string
  action_config_preview: string | null
  result_preview: string | null
}

/** Full task detail — returned by GET /api/v1/tasks/:id with untruncated fields. */
export interface TaskDetailItem {
  id: string
  agent_id: string
  label: string
  trigger_type: string
  action_type: string
  status: string
  team_run_id: string | null
  parent_task_id: string | null
  depth: number
  source: string | null
  reference_url: string | null
  cron_expr: string | null
  next_fire_at: string | null
  fired_at: string | null
  completed_at: string | null
  created_by_session: string | null
  created_trace_id: string | null
  execution_trace_id: string | null
  created_at: string
  updated_at: string
  action_config: string
  result: string | null
}

export interface TasksFilters {
  status?: string
  trigger_type?: string
  action_type?: string
  agent_id?: string
  team_run_id?: string
  source?: string
  page?: number
  per_page?: number
}

export function useTasks(filters: TasksFilters) {
  return useQuery<PaginatedResponse<TaskItem>>({
    queryKey: ['tasks', filters],
    queryFn: () =>
      apiFetch('/tasks', filters as Record<string, string | number | undefined>),
  })
}

export function useTask(taskId: string | undefined) {
  return useQuery<TaskDetailItem>({
    queryKey: ['task', taskId],
    queryFn: () => apiFetch(`/tasks/${taskId}`),
    enabled: !!taskId,
  })
}

export function useTaskChildren(parentTaskId: string | undefined) {
  return useQuery<TaskItem[]>({
    queryKey: ['task-children', parentTaskId],
    queryFn: () => apiFetch(`/tasks/${parentTaskId}/children`),
    enabled: !!parentTaskId,
  })
}

/** Session linked to a task tree — returned by GET /api/v1/tasks/:id/sessions. */
export interface TaskSession {
  id: string
  agent_id: string
  channel_type: string
  started_at: string
  ended_at: string | null
  task_id: string | null
  message_count: number
  task_label: string | null
}

export interface TaskSessionsResponse {
  sessions: TaskSession[]
}

export function useTaskSessions(taskId: string | undefined) {
  return useQuery<TaskSessionsResponse>({
    queryKey: ['task-sessions', taskId],
    queryFn: () => apiFetch(`/tasks/${taskId}/sessions`),
    enabled: !!taskId,
  })
}
