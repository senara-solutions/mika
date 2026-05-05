import { useState } from 'react'
import { Link, useParams } from 'react-router'
import { useLlmCall, useLlmCallToolCalls } from '../api/llmCalls.ts'
import { CopyButton, StatusBadge, EmptyState, LoadingState, ErrorState, formatApiError, formatRelativeTime } from '@senara-solutions/ui'
import type { StatusBadgeVariant } from '@senara-solutions/ui'
import { MetadataRow } from '../components/MetadataRow.tsx'

function formatTokens(n: number | null): string {
  if (n == null) return '-'
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${ms}ms`
}

function llmStatusVariant(status: string): { variant: StatusBadgeVariant; label: string } {
  switch (status) {
    case 'success': return { variant: 'success', label: 'Success' }
    case 'error': return { variant: 'error', label: 'Error' }
    default: return { variant: 'neutral', label: status }
  }
}

export default function LlmCallDetail() {
  const { id } = useParams<{ id: string }>()
  const { data: call, isLoading, error, refetch } = useLlmCall(id)
  const { data: toolCalls } = useLlmCallToolCalls(call?.id)
  const [reasoningExpanded, setReasoningExpanded] = useState(false)

  if (isLoading) {
    return <LoadingState variant="detail" />
  }
  if (error) {
    return <ErrorState message={formatApiError(error)} retry={() => refetch()} />
  }
  if (!call) {
    return <EmptyState message="LLM call not found" />
  }

  return (
    <div>
      <div className="mb-5">
        <Link to="/llm-calls" className="text-muted/60 text-xs hover:text-muted transition-colors">
          &larr; Back to LLM Calls
        </Link>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h2 className="text-heading text-xl font-semibold">
            {call.provider} / {call.model}
          </h2>
          <div className="flex items-center gap-3 mt-2">
            <StatusBadge {...llmStatusVariant(call.status)} />
            <CopyButton text={call.id} title="Copy ID" />
          </div>
        </div>
      </div>

      {/* Error Banner — prominent when status is error */}
      {call.status === 'error' && (
        <div className="bg-red-500/[0.06] border border-red-400/20 rounded-2xl p-5 mb-4">
          <div className="flex items-center gap-2 mb-3">
            <svg className="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
            </svg>
            <h3 className="text-red-400 text-sm font-medium">LLM Call Failed</h3>
            {call.error_message && (
              <CopyButton text={call.error_message} title="Copy error" />
            )}
          </div>
          <pre className="font-mono text-xs text-red-300/80 whitespace-pre-wrap break-all max-h-64 overflow-y-auto">
            {call.error_message || 'Unknown error'}
          </pre>
        </div>
      )}

      {/* Call Metadata */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Call Details</h3>
        <MetadataRow label="ID">
          <span className="font-mono text-xs">{call.id}</span>
        </MetadataRow>
        <MetadataRow label="Provider">{call.provider}</MetadataRow>
        <MetadataRow label="Model">
          <span className="font-mono text-xs">{call.model}</span>
        </MetadataRow>
        <MetadataRow label="Step">{call.step}</MetadataRow>
        <MetadataRow label="Latency">{formatLatency(call.latency_ms)}</MetadataRow>
        <MetadataRow label="Stop Reason">
          <span className="font-mono text-xs">{call.stop_reason ?? '\u2014'}</span>
        </MetadataRow>
      </div>

      {/* Response Content */}
      {call.response_text && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-heading text-sm font-medium">Response</h3>
            <CopyButton text={call.response_text} title="Copy response" />
          </div>
          <pre className="font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {call.response_text}
          </pre>
        </div>
      )}

      {/* Reasoning / Extended Thinking */}
      {call.reasoning && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <button
            onClick={() => setReasoningExpanded(!reasoningExpanded)}
            className="flex items-center gap-2 w-full text-left"
          >
            <svg
              className={`w-3 h-3 text-muted/50 transition-transform ${reasoningExpanded ? 'rotate-90' : ''}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
            <h3 className="text-heading text-sm font-medium">Reasoning</h3>
            {reasoningExpanded && <CopyButton text={call.reasoning} title="Copy reasoning" />}
          </button>
          {reasoningExpanded && (
            <pre className="font-mono text-xs text-muted/50 whitespace-pre-wrap break-all max-h-96 overflow-y-auto mt-3">
              {call.reasoning}
            </pre>
          )}
        </div>
      )}

      {/* Linked Tool Calls */}
      {toolCalls && toolCalls.length > 0 && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <h3 className="text-heading text-sm font-medium mb-3">
            Tool Calls ({toolCalls.length})
          </h3>
          <div className="space-y-2">
            {toolCalls.map((tc) => (
              <Link
                key={tc.id}
                to={`/tool-calls/${tc.id}`}
                className="flex items-center justify-between p-3 rounded-xl bg-bg hover:bg-white/[0.03] border border-white/[0.04] transition-colors group"
              >
                <div className="flex items-center gap-3">
                  <StatusBadge
                    variant={tc.success ? 'success' : 'error'}
                    label={tc.success ? 'OK' : 'Fail'}
                  />
                  <span className="font-mono text-xs text-heading group-hover:text-accent transition-colors">
                    {tc.tool_name}
                  </span>
                  {tc.skill_name && (
                    <span className="text-[10px] text-muted/40">{tc.skill_name}</span>
                  )}
                </div>
                <span className="text-xs text-muted/50">{formatLatency(tc.latency_ms)}</span>
              </Link>
            ))}
          </div>
        </div>
      )}

      {/* Skill Variants */}
      {call.prompt_variant && (() => {
        try {
          const variants = JSON.parse(call.prompt_variant) as Record<string, string>
          return (
            <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
              <h3 className="text-heading text-sm font-medium mb-3">Skill Variants</h3>
              {Object.entries(variants).map(([skill, variant]) => (
                <MetadataRow key={skill} label={skill}>
                  <span className="font-mono text-xs">{variant}</span>
                </MetadataRow>
              ))}
            </div>
          )
        } catch {
          return (
            <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
              <h3 className="text-heading text-sm font-medium mb-3">Skill Variants</h3>
              <MetadataRow label="Raw">
                <span className="font-mono text-xs">{call.prompt_variant}</span>
              </MetadataRow>
            </div>
          )
        }
      })()}

      {/* Token Usage */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">Token Usage</h3>
        <MetadataRow label="Input">{formatTokens(call.input_tokens)}</MetadataRow>
        <MetadataRow label="Output">{formatTokens(call.output_tokens)}</MetadataRow>
        <MetadataRow label="Cache Read">{formatTokens(call.cache_read_tokens)}</MetadataRow>
        <MetadataRow label="Cache Write">{formatTokens(call.cache_write_tokens)}</MetadataRow>
      </div>

      {/* References */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4 space-y-1">
        <h3 className="text-heading text-sm font-medium mb-3">References</h3>
        <MetadataRow label="Agent">
          <Link
            to={`/agents/${call.agent_id}`}
            className="text-accent text-xs hover:text-accent-light transition-colors"
          >
            {call.agent_id}
          </Link>
        </MetadataRow>
        <MetadataRow label="Session">
          <Link
            to={`/sessions/${call.session_id}`}
            className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
          >
            {call.session_id}
          </Link>
        </MetadataRow>
        {call.trace_id && (
          <MetadataRow label="Trace">
            <Link
              to={`/traces/${call.trace_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {call.trace_id.slice(0, 12)}...
            </Link>
          </MetadataRow>
        )}
        <MetadataRow label="Timestamp">{formatRelativeTime(call.created_at)}</MetadataRow>
      </div>
    </div>
  )
}
