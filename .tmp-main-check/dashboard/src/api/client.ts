export const API_BASE = '/api/v1'

/** Resolve the auth token at runtime.
 *  Priority: server-injected window.__MIKA_CONFIG__ (embedded mode) > Vite env var (dev mode).
 */
function getToken(): string {
  if (window.__MIKA_CONFIG__?.token) {
    return window.__MIKA_CONFIG__.token
  }
  return import.meta.env.VITE_MIKA_DASHBOARD_TOKEN ?? ''
}

export const TOKEN = getToken()

export interface PaginatedResponse<T> {
  data: T[]
  total: number
  page: number
  per_page: number
}

export async function apiFetch<T>(
  path: string,
  params?: Record<string, string | number | undefined>,
): Promise<T> {
  const url = new URL(`${API_BASE}${path}`, window.location.origin)
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined && value !== '') {
        url.searchParams.set(key, String(value))
      }
    }
  }

  const res = await fetch(url.toString(), {
    headers: {
      ...(TOKEN ? { Authorization: `Bearer ${TOKEN}` } : {}),
    },
  })

  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }))
    throw new Error(body.error ?? `HTTP ${res.status}`)
  }

  return res.json()
}
