import { useState, useRef, useEffect, useCallback } from 'react'
import { X, Search, Send, Loader2, Wrench, Trash2, MessageSquare, Cpu } from 'lucide-react'
import { streamInvestigation, type InvestigateEvent } from '../api/investigate.ts'
import CopyButton from './CopyButton.tsx'
import {
  type InvestigationScope,
  type ChatMessage,
  buildScopeKey,
  loadInvestigation,
  saveInvestigation,
  clearInvestigation,
  clearAllInvestigations,
} from '../lib/investigationStorage.ts'

export type { InvestigationScope }

export default function InvestigationPanel({
  scope,
  onClose,
}: {
  scope: InvestigationScope
  onClose: () => void
}) {
  const scopeKey = buildScopeKey(scope)
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [restored, setRestored] = useState(false)
  const [input, setInput] = useState('')
  const [streaming, setStreaming] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [showClearMenu, setShowClearMenu] = useState(false)
  const abortRef = useRef<AbortController | null>(null)
  const messagesRef = useRef<ChatMessage[]>([])
  const scrollRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  // Keep messagesRef in sync
  useEffect(() => { messagesRef.current = messages }, [messages])

  // Load history from localStorage on mount
  useEffect(() => {
    const saved = loadInvestigation(scopeKey)
    if (saved && saved.length > 0) {
      setMessages(saved)
      setRestored(true)
    }
  }, [scopeKey])

  // Auto-scroll on new content
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: 'smooth' })
  }, [messages])

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  // Escape to close
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setShowClearMenu(false)
        onClose()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  // Cleanup abort on unmount
  useEffect(() => {
    return () => {
      abortRef.current?.abort()
    }
  }, [])

  // Close clear menu on click outside
  useEffect(() => {
    if (!showClearMenu) return
    const handler = () => setShowClearMenu(false)
    window.addEventListener('click', handler)
    return () => window.removeEventListener('click', handler)
  }, [showClearMenu])

  const friendlyError = (msg: string): string => {
    if (msg.includes('429') || msg.toLowerCase().includes('already running') || msg.toLowerCase().includes('busy')) {
      return 'Previous investigation is still winding down, please try again in a moment.'
    }
    return msg
  }

  const sendQuestion = useCallback(
    async (question: string) => {
      if (!question.trim() || streaming) return

      setError(null)
      setStreaming(true)
      setRestored(false)

      // Add user message using functional update to avoid stale closure
      const userMsg: ChatMessage = { role: 'user', content: question }
      // Capture history from ref before state update
      const history = messagesRef.current.map((m) => ({
        role: m.role,
        content: m.content,
      }))
      setMessages((prev) => [...prev, userMsg])
      setInput('')
      if (inputRef.current) {
        inputRef.current.style.height = 'auto'
      }

      // Start streaming
      const controller = new AbortController()
      abortRef.current = controller

      let assistantContent = ''
      const toolUses: { name: string; status: string; summary?: string }[] = []

      const handleEvent = (event: InvestigateEvent) => {
        switch (event.type) {
          case 'text_delta':
            assistantContent += event.text
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === 'assistant') {
                updated[updated.length - 1] = {
                  ...last,
                  content: assistantContent,
                  toolUses: [...toolUses],
                }
              } else {
                updated.push({
                  role: 'assistant',
                  content: assistantContent,
                  toolUses: [...toolUses],
                })
              }
              return updated
            })
            break
          case 'tool_use':
            if (event.status === 'running') {
              toolUses.push({ name: event.name, status: 'running' })
            } else {
              const existing = toolUses.find(
                (t) => t.name === event.name && t.status === 'running',
              )
              if (existing) {
                existing.status = 'completed'
                existing.summary = event.summary
              }
            }
            setMessages((prev) => {
              const updated = [...prev]
              const last = updated[updated.length - 1]
              if (last?.role === 'assistant') {
                updated[updated.length - 1] = {
                  ...last,
                  content: assistantContent,
                  toolUses: [...toolUses],
                }
              } else {
                updated.push({
                  role: 'assistant',
                  content: assistantContent || '...',
                  toolUses: [...toolUses],
                })
              }
              return updated
            })
            break
          case 'error':
            setError(friendlyError(event.message))
            break
          case 'done':
            // Persist to localStorage after complete response
            saveInvestigation(scope, messagesRef.current)
            break
        }
      }

      if (scope.messageId == null) {
        setError('No message available for investigation')
        setStreaming(false)
        return
      }

      try {
        await streamInvestigation(
          {
            message_id: scope.messageId,
            tool_call_index: scope.toolCallIndex,
            question,
            history,
          },
          handleEvent,
          controller.signal,
        )
      } catch (e) {
        if (e instanceof DOMException && e.name === 'AbortError') return
        setError(friendlyError(e instanceof Error ? e.message : 'Investigation failed'))
      } finally {
        setStreaming(false)
        abortRef.current = null
      }
    },
    [scope, streaming],
  )

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    sendQuestion(input)
  }

  const handleClearCurrent = () => {
    clearInvestigation(scopeKey)
    setMessages([])
    setRestored(false)
    setShowClearMenu(false)
  }

  const handleClearAll = () => {
    clearAllInvestigations()
    setMessages([])
    setRestored(false)
    setShowClearMenu(false)
  }

  // Build scope label
  const scopeLabel = (() => {
    if (scope.type === 'session') return 'Full Session'
    if (scope.type === 'tool_call' && scope.toolName) {
      return `Tool: ${scope.toolName} (step ${(scope.toolCallIndex ?? 0) + 1})`
    }
    return `Message #${scope.messageId}`
  })()

  const scopeIcon = scope.type === 'session' ? (
    <MessageSquare size={10} />
  ) : scope.type === 'tool_call' ? (
    <Wrench size={10} />
  ) : (
    <Cpu size={10} />
  )

  return (
    <>
      {/* Backdrop */}
      <div className="fixed inset-0 bg-black/30 z-40" onClick={onClose} />

      {/* Panel */}
      <div className="fixed top-0 right-0 h-full w-[40%] min-w-[400px] max-w-[600px] bg-bg border-l border-white/[0.06] z-50 flex flex-col shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/[0.06]">
          <div className="flex items-center gap-2">
            <Search size={14} className="text-accent" />
            <span className="text-sm font-semibold text-heading">Investigation</span>
          </div>
          <div className="flex items-center gap-1">
            {/* Clear menu */}
            <div className="relative">
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  setShowClearMenu((v) => !v)
                }}
                className="p-1 rounded hover:bg-white/[0.05] text-muted/40 hover:text-muted/70 transition-colors"
                title="Clear history"
              >
                <Trash2 size={14} />
              </button>
              {showClearMenu && (
                <div className="absolute right-0 top-full mt-1 bg-bg-card border border-white/[0.08] rounded-lg shadow-xl py-1 z-50 min-w-[180px]">
                  <button
                    onClick={handleClearCurrent}
                    className="w-full text-left px-3 py-1.5 text-xs text-muted/70 hover:bg-white/[0.05] hover:text-heading transition-colors"
                  >
                    Clear this investigation
                  </button>
                  <button
                    onClick={handleClearAll}
                    className="w-full text-left px-3 py-1.5 text-xs text-red-400/70 hover:bg-red-400/5 hover:text-red-400 transition-colors"
                  >
                    Clear all investigations
                  </button>
                </div>
              )}
            </div>
            <button
              onClick={onClose}
              className="p-1 rounded hover:bg-white/[0.05] text-muted/60 transition-colors"
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* Scope badge */}
        <div className="px-4 py-2 border-b border-white/[0.04] bg-white/[0.02]">
          <div className="flex items-center gap-2">
            <div className="text-[10px] text-muted/40 uppercase tracking-wider">Investigating</div>
            <span className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full bg-accent/10 text-accent/80 font-medium">
              {scopeIcon}
              {scopeLabel}
            </span>
          </div>
          <div className="text-[10px] text-muted/30 mt-0.5">
            Session: {scope.sessionId.slice(0, 12)}... &middot; Agent: {scope.agentId}
          </div>
        </div>

        {/* Messages */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
          {/* Restored indicator */}
          {restored && messages.length > 0 && (
            <div className="flex items-center gap-2 text-[10px] text-muted/30">
              <div className="flex-1 border-t border-white/[0.04]" />
              <span>Restored from previous investigation</span>
              <div className="flex-1 border-t border-white/[0.04]" />
            </div>
          )}

          {messages.length === 0 && !streaming && (
            <div className="text-center text-muted/30 text-sm py-12">
              Ask a question about this agent's behavior
            </div>
          )}

          {messages.map((msg, i) => (
            <div key={i} className={msg.role === 'user' ? 'flex justify-end' : ''}>
              <div
                className={
                  msg.role === 'user'
                    ? 'bg-accent/10 border border-accent/20 rounded-xl px-3 py-2 max-w-[85%]'
                    : 'bg-white/[0.03] border border-white/[0.06] rounded-xl px-3 py-2'
                }
              >
                {/* Tool use badges */}
                {msg.toolUses && msg.toolUses.length > 0 && (
                  <div className="flex flex-wrap gap-1.5 mb-2">
                    {msg.toolUses.map((tu, j) => (
                      <span
                        key={j}
                        className={`inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-mono ${
                          tu.status === 'running'
                            ? 'bg-amber-400/10 text-amber-400/70'
                            : 'bg-emerald-400/10 text-emerald-400/70'
                        }`}
                      >
                        <Wrench size={9} />
                        {tu.name}
                        {tu.status === 'running' && (
                          <Loader2 size={9} className="animate-spin" />
                        )}
                        {tu.status === 'completed' && ' ✓'}
                      </span>
                    ))}
                  </div>
                )}

                <div className="text-sm text-muted/80 whitespace-pre-wrap break-words">
                  {msg.content}
                </div>
                {msg.role === 'assistant' && msg.content && (
                  <div className="flex justify-end mt-1">
                    <CopyButton text={msg.content} title="Copy response" />
                  </div>
                )}
              </div>
            </div>
          ))}

          {streaming && messages[messages.length - 1]?.role !== 'assistant' && (
            <div className="flex items-center gap-2 text-muted/40 text-xs">
              <Loader2 size={12} className="animate-spin" />
              Thinking...
            </div>
          )}

          {error && (
            <div className="bg-red-400/10 border border-red-400/20 rounded-lg px-3 py-2 text-xs text-red-400">
              {error}
            </div>
          )}
        </div>

        {/* Input */}
        <form
          onSubmit={handleSubmit}
          className="border-t border-white/[0.06] px-4 py-3 flex items-end gap-2"
        >
          <textarea
            ref={inputRef}
            rows={1}
            value={input}
            onChange={(e) => {
              setInput(e.target.value)
              e.target.style.height = 'auto'
              e.target.style.height = e.target.scrollHeight + 'px'
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault()
                if (input.trim()) sendQuestion(input)
              }
            }}
            placeholder="Ask a question..."
            disabled={streaming}
            className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-heading placeholder:text-muted/30 focus:outline-none focus:border-accent/40 disabled:opacity-50 resize-none overflow-y-auto max-h-[120px]"
          />
          <button
            type="submit"
            disabled={streaming || !input.trim()}
            className="p-2 rounded-lg bg-accent/20 text-accent hover:bg-accent/30 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          >
            {streaming ? <Loader2 size={16} className="animate-spin" /> : <Send size={16} />}
          </button>
        </form>
      </div>
    </>
  )
}
