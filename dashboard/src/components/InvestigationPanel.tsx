import { useState, useRef, useEffect, useCallback } from 'react'
import { X, Search, Send, Loader2, Wrench } from 'lucide-react'
import { streamInvestigation, type InvestigateEvent } from '../api/investigate.ts'
import CopyButton from './CopyButton.tsx'

export interface InvestigationContext {
  messageId: number
  toolCallIndex?: number
  toolName?: string
  sessionId: string
  agentId: string
}

interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  toolUses?: { name: string; status: string; summary?: string }[]
}

export default function InvestigationPanel({
  context,
  onClose,
}: {
  context: InvestigationContext
  onClose: () => void
}) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [input, setInput] = useState('')
  const [streaming, setStreaming] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const abortRef = useRef<AbortController | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

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
      if (e.key === 'Escape') onClose()
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

  const sendQuestion = useCallback(
    async (question: string) => {
      if (!question.trim() || streaming) return

      setError(null)
      setStreaming(true)

      // Add user message
      const userMsg: ChatMessage = { role: 'user', content: question }
      setMessages((prev) => [...prev, userMsg])
      setInput('')

      // Build history from previous messages
      const history = messages.map((m) => ({
        role: m.role,
        content: m.content,
      }))

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
            // Update the assistant message with tool use info
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
            setError(event.message)
            break
          case 'done':
            break
        }
      }

      try {
        await streamInvestigation(
          {
            message_id: context.messageId,
            tool_call_index: context.toolCallIndex,
            question,
            history,
          },
          handleEvent,
          controller.signal,
        )
      } catch (e) {
        if (e instanceof DOMException && e.name === 'AbortError') return
        setError(e instanceof Error ? e.message : 'Investigation failed')
      } finally {
        setStreaming(false)
        abortRef.current = null
      }
    },
    [context, messages, streaming],
  )

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    sendQuestion(input)
  }

  const contextLabel = context.toolName
    ? `${context.toolName} (step ${(context.toolCallIndex ?? 0) + 1})`
    : 'Message'

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
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-white/[0.05] text-muted/60 transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        {/* Context bar */}
        <div className="px-4 py-2 border-b border-white/[0.04] bg-white/[0.02]">
          <div className="text-[10px] text-muted/40 uppercase tracking-wider">Investigating</div>
          <div className="text-xs text-muted/70 font-mono mt-0.5">{contextLabel}</div>
          <div className="text-[10px] text-muted/30 mt-0.5">
            Session: {context.sessionId.slice(0, 12)}... &middot; Agent: {context.agentId}
          </div>
        </div>

        {/* Messages */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
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
                    : 'bg-white/[0.03] border border-white/[0.06] rounded-xl px-3 py-2 relative group'
                }
              >
                {msg.role === 'assistant' && msg.content && (
                  <div className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 transition-opacity">
                    <CopyButton text={msg.content} title="Copy response" />
                  </div>
                )}
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
          className="border-t border-white/[0.06] px-4 py-3 flex items-center gap-2"
        >
          <input
            ref={inputRef}
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Ask a question..."
            disabled={streaming}
            className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-heading placeholder:text-muted/30 focus:outline-none focus:border-accent/40 disabled:opacity-50"
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
