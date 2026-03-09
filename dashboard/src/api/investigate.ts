const API_BASE = '/api/v1'
const TOKEN = import.meta.env.VITE_MIKA_TOKEN ?? ''

export interface InvestigateRequest {
  message_id: number
  tool_call_index?: number
  question: string
  history?: { role: string; content: string }[]
}

export type InvestigateEvent =
  | { type: 'text_delta'; text: string }
  | { type: 'tool_use'; name: string; status: 'running' | 'completed'; summary?: string }
  | { type: 'done' }
  | { type: 'error'; message: string }

/**
 * Stream an investigation request via SSE.
 * Uses fetch + ReadableStream (not EventSource) for Bearer auth support.
 */
export async function streamInvestigation(
  req: InvestigateRequest,
  onEvent: (event: InvestigateEvent) => void,
  signal: AbortSignal,
): Promise<void> {
  const res = await fetch(`${API_BASE}/investigate`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(TOKEN ? { Authorization: `Bearer ${TOKEN}` } : {}),
    },
    body: JSON.stringify(req),
    signal,
  })

  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }))
    throw new Error(body.error ?? `HTTP ${res.status}`)
  }

  if (!res.body) {
    throw new Error('No response body')
  }

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break

      buffer += decoder.decode(value, { stream: true })
      const chunks = buffer.split('\n\n')
      buffer = chunks.pop() ?? ''

      for (const chunk of chunks) {
        if (!chunk.trim()) continue

        let eventType = 'message'
        let data = ''

        for (const line of chunk.split('\n')) {
          if (line.startsWith('event: ')) eventType = line.slice(7)
          else if (line.startsWith('data: ')) data += line.slice(6)
          else if (line.startsWith(':')) continue // keep-alive comment
        }

        if (!data) continue

        try {
          const parsed = JSON.parse(data) as InvestigateEvent
          // Override type from the event field if present
          if (eventType === 'text_delta') parsed.type = 'text_delta'
          else if (eventType === 'tool_use') parsed.type = 'tool_use'
          else if (eventType === 'done') parsed.type = 'done'
          else if (eventType === 'error') parsed.type = 'error'
          onEvent(parsed)
        } catch {
          // Skip unparseable events
        }
      }
    }
  } finally {
    reader.releaseLock()
  }
}
