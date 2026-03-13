const STORAGE_KEY = 'mika:investigations'
const MAX_ENTRIES = 20
const MAX_AGE_MS = 14 * 24 * 60 * 60 * 1000 // 14 days

export interface InvestigationScope {
  type: 'message' | 'tool_call' | 'session'
  sessionId: string
  agentId: string
  messageId?: number
  toolCallIndex?: number
  toolName?: string
}

export interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  toolUses?: { name: string; status: string; summary?: string }[]
}

interface StoredInvestigation {
  scopeKey: string
  messages: ChatMessage[]
  lastUpdatedAt: number
  scope: InvestigationScope
}

interface InvestigationStore {
  version: 1
  entries: StoredInvestigation[]
}

export function buildScopeKey(scope: InvestigationScope): string {
  const base = `sess:${scope.sessionId}:agent:${scope.agentId}`
  if (scope.type === 'session') return `${base}:session`
  if (scope.messageId == null) return `${base}:session`
  if (scope.type === 'tool_call' && scope.toolCallIndex != null) {
    return `${base}:msg:${scope.messageId}:tool:${scope.toolCallIndex}`
  }
  return `${base}:msg:${scope.messageId}`
}

function readStore(): InvestigationStore {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { version: 1, entries: [] }
    const parsed = JSON.parse(raw)
    if (parsed?.version !== 1 || !Array.isArray(parsed.entries)) {
      return { version: 1, entries: [] }
    }
    return parsed as InvestigationStore
  } catch {
    return { version: 1, entries: [] }
  }
}

function writeStore(store: InvestigationStore): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store))
  } catch {
    // QuotaExceededError or other — silently ignore
  }
}

function prune(store: InvestigationStore): InvestigationStore {
  const now = Date.now()
  // Remove entries older than 14 days
  let entries = store.entries.filter((e) => now - e.lastUpdatedAt < MAX_AGE_MS)
  // Keep only the 20 most recent
  if (entries.length > MAX_ENTRIES) {
    entries.sort((a, b) => b.lastUpdatedAt - a.lastUpdatedAt)
    entries = entries.slice(0, MAX_ENTRIES)
  }
  return { version: 1, entries }
}

export function loadInvestigation(scopeKey: string): ChatMessage[] | null {
  const store = readStore()
  const entry = store.entries.find((e) => e.scopeKey === scopeKey)
  return entry?.messages ?? null
}

export function saveInvestigation(scope: InvestigationScope, messages: ChatMessage[]): void {
  const scopeKey = buildScopeKey(scope)
  const store = readStore()
  const existing = store.entries.findIndex((e) => e.scopeKey === scopeKey)
  const entry: StoredInvestigation = {
    scopeKey,
    messages,
    lastUpdatedAt: Date.now(),
    scope,
  }
  if (existing >= 0) {
    store.entries[existing] = entry
  } else {
    store.entries.push(entry)
  }
  writeStore(prune(store))
}

export function clearInvestigation(scopeKey: string): void {
  const store = readStore()
  store.entries = store.entries.filter((e) => e.scopeKey !== scopeKey)
  writeStore(store)
}

export function clearAllInvestigations(): void {
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    // silently ignore
  }
}
