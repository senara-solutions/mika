import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, TOKEN, API_BASE } from './client'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface VariantSummary {
  skill_name: string
  total: number
  hand_authored: number
  generated: number
  experimental: number
  stale_count: number
}

export interface VariantMetadata {
  skill_name: string
  provider: string
  model: string
  source: 'hand_authored' | 'generated' | 'experimental'
  size_bytes: number
  line_count: number
  last_modified: string | null
  stale: boolean | null
}

export interface DiffReport {
  classification: 'cosmetic' | 'content' | 'structural' | 'identical'
  added_lines: number
  removed_lines: number
  missing_base_sections: string[]
  unified_diff: string
}

export interface ValidationResult {
  rule: string
  passed: boolean
  message: string
  level: 'ok' | 'warn' | 'fail'
}

export interface ValidationResponse {
  skill: string
  provider: string
  model: string
  passed: boolean
  results: ValidationResult[]
}

export interface VariantImpact {
  metadata: VariantMetadata
  diff_summary: {
    classification: string
    added_lines: number
    removed_lines: number
    missing_sections: string[]
  }
  recommendation: string
}

export interface ReflectionReport {
  skill_name: string
  base_last_modified: string | null
  impacts: VariantImpact[]
}

export interface PromoteResponse {
  promoted: boolean
  source: string
  target: string
  warnings: string[]
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

export function useVariantsSummary() {
  return useQuery<VariantSummary[]>({
    queryKey: ['variants-summary'],
    queryFn: () => apiFetch<VariantSummary[]>('/skills/variants'),
  })
}

export function useSkillVariants(skillName: string | null) {
  return useQuery<VariantMetadata[]>({
    queryKey: ['skill-variants', skillName],
    queryFn: () => apiFetch<VariantMetadata[]>(`/skills/${skillName}/variants`),
    enabled: !!skillName,
  })
}

export function useVariantDiff(skillName: string, provider: string, model: string, enabled: boolean) {
  return useQuery<DiffReport>({
    queryKey: ['variant-diff', skillName, provider, model],
    queryFn: () => apiFetch<DiffReport>(`/skills/${skillName}/variants/${provider}/${model}/diff`),
    enabled,
  })
}

export function useVariantReflect(skillName: string | null, enabled: boolean) {
  return useQuery<ReflectionReport>({
    queryKey: ['variant-reflect', skillName],
    queryFn: () => apiFetch<ReflectionReport>(`/skills/${skillName}/variants/reflect`),
    enabled: !!skillName && enabled,
  })
}

export function useValidateVariant() {
  return useMutation<ValidationResponse, Error, { skill: string; provider: string; model: string }>({
    mutationFn: async ({ skill, provider, model }) => {
      const url = `${API_BASE}/skills/${skill}/variants/${provider}/${model}/validate`
      const res = await fetch(url, {
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
  })
}

export function usePromoteVariant() {
  const queryClient = useQueryClient()
  return useMutation<PromoteResponse, Error, { skill: string; provider: string; model: string }>({
    mutationFn: async ({ skill, provider, model }) => {
      const url = `${API_BASE}/skills/${skill}/variants/${provider}/${model}/promote`
      const res = await fetch(url, {
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
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: ['skill-variants', variables.skill] })
      queryClient.invalidateQueries({ queryKey: ['variants-summary'] })
    },
  })
}
