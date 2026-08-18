import { Link, useParams } from 'react-router'
import { useToolCall } from '../api/toolCalls.ts'
import { CopyButton, StatusBadge, EmptyState, LoadingState, ErrorState, formatApiError, formatRelativeTime } from '@samidarko/ui'
import { MetadataRow } from '../components/MetadataRow.tsx'

function formatLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`
  return `${ms}ms`
}

function sourceBadge(source: string) {
  switch (source) {
    case 'builtin':
      return 'bg-accent/15 text-accent/80'
    case 'skill':
      return 'bg-purple-400/15 text-purple-400'
    case 'mcp':
      return 'bg-amber-400/15 text-amber-400'
    default:
      return 'bg-white/[0.06] text-muted/60'
  }
}

export default function ToolCallDetail() {
  const { id } = useParams<{ id: string }>()
  const { data: call, isLoading, error, refetch } = useToolCall(id)

  if (isLoading) {
    return <LoadingState variant="detail" />
  }
  if (error) {
    return <ErrorState message={formatApiError(error)} retry={() => refetch()} />
  }
  if (!call) {
    return <EmptyState message="Tool call not found" />
  }

  return (
    <div>
      <div className="mb-5">
        <Link to="/tool-calls" className="text-muted/60 text-xs hover:text-muted transition-colors">
          &larr; Back to Tool Calls
        </Link>
      </div>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h2 className="text-heading text-xl font-semibold font-mono">{call.tool_name}</h2>
          <div className="flex items-center gap-3 mt-2">
            <StatusBadge variant={call.success ? 'success' : 'error'} label={call.success ? 'Success' : 'Failed'} />
            <span className={`inline-flex items-center text-[10px] font-semibold px-2 py-0.5 rounded-full ${sourceBadge(call.tool_source)}`}>
              {call.tool_source}
            </span>
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
        <MetadataRow label="Tool">
          <span className="font-mono text-xs">{call.tool_name}</span>
        </MetadataRow>
        <MetadataRow label="Source">
          <span className={`inline-flex items-center text-[10px] font-semibold px-2 py-0.5 rounded-full ${sourceBadge(call.tool_source)}`}>
            {call.tool_source}
          </span>
        </MetadataRow>
        {call.skill_name && (
          <MetadataRow label="Skill">{call.skill_name}</MetadataRow>
        )}
        <MetadataRow label="Step">{call.step}</MetadataRow>
        <MetadataRow label="Latency">{formatLatency(call.latency_ms)}</MetadataRow>
        {call.non_zero_exit && (
          <MetadataRow label="Exit">
            <span className="text-amber-400 text-xs">non-zero exit</span>
          </MetadataRow>
        )}
        {call.error_message && (
          <MetadataRow label="Error">
            <span className="text-red-400 text-xs">{call.error_message}</span>
          </MetadataRow>
        )}
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
        {call.llm_call_id && (
          <MetadataRow label="LLM Call">
            <Link
              to={`/llm-calls/${call.llm_call_id}`}
              className="text-accent text-xs font-mono hover:text-accent-light transition-colors"
            >
              {call.llm_call_id}
            </Link>
          </MetadataRow>
        )}
        <MetadataRow label="Timestamp">{formatRelativeTime(call.created_at)}</MetadataRow>
      </div>

      {/* Input */}
      {call.input && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-heading text-sm font-medium">Input</h3>
            <CopyButton text={call.input} title="Copy input" />
          </div>
          <pre className="font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {call.input}
          </pre>
        </div>
      )}

      {/* Output */}
      {call.output && (
        <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 mt-4">
          <div className="flex items-center gap-2 mb-3">
            <h3 className="text-heading text-sm font-medium">Output</h3>
            <CopyButton text={call.output} title="Copy output" />
          </div>
          <pre className="font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto">
            {call.output}
          </pre>
        </div>
      )}
    </div>
  )
}
