import { Link, useParams } from 'react-router'
import { useLlmCall } from '../api/llmCalls.ts'
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
          <span className="font-mono text-xs">{call.stop_reason ?? '—'}</span>
        </MetadataRow>
        {call.error_message && (
          <MetadataRow label="Error">
            <span className="text-red-400 text-xs">{call.error_message}</span>
          </MetadataRow>
        )}
      </div>

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
