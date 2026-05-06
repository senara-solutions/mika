import { useState } from 'react'
import { CopyButton } from '@senara-solutions/ui'
import { Link } from 'react-router'

const TRUNCATION_LIMIT = 10_000

function tryFormatJson(text: string): { formatted: string; isJson: boolean } {
  try {
    const parsed = JSON.parse(text)
    if (typeof parsed === 'object' && parsed !== null) {
      return { formatted: JSON.stringify(parsed, null, 2), isJson: true }
    }
  } catch {
    // Not valid JSON — fall back to raw text
  }
  return { formatted: text, isJson: false }
}

interface BodySectionProps {
  label: string
  text: string
  defaultExpanded?: boolean
}

function BodySection({ label, text, defaultExpanded = true }: BodySectionProps) {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded)
  const [showFull, setShowFull] = useState(false)

  const isTruncated = text.length > TRUNCATION_LIMIT && !showFull
  const displayText = isTruncated ? text.slice(0, TRUNCATION_LIMIT) : text
  const { formatted, isJson } = tryFormatJson(displayText)

  return (
    <div className="bg-bg border border-white/[0.05] rounded-xl p-4">
      <div className="flex items-center gap-2">
        <button
          onClick={() => setIsExpanded(!isExpanded)}
          className="flex items-center gap-2 text-left"
          aria-expanded={isExpanded}
          aria-label={`${isExpanded ? 'Collapse' : 'Expand'} ${label}`}
        >
          <svg
            className={`w-3 h-3 text-muted/50 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          <h4 className="text-heading text-xs font-medium">{label}</h4>
        </button>
        {isExpanded && <CopyButton text={text} title={`Copy ${label.toLowerCase()}`} />}
      </div>
      {isExpanded && (
        <div className="mt-3">
          <pre className={`font-mono text-xs whitespace-pre-wrap break-all max-h-96 overflow-y-auto ${isJson ? 'text-muted/80' : 'text-muted/70'}`}>
            {formatted}
          </pre>
          {text.length > TRUNCATION_LIMIT && (
            <div className="flex items-center gap-2 mt-2 pt-2 border-t border-white/[0.05]">
              <span className="text-[10px] text-muted/40">
                {isTruncated
                  ? `Showing ${TRUNCATION_LIMIT.toLocaleString()} of ${text.length.toLocaleString()} characters`
                  : `${text.length.toLocaleString()} characters`}
              </span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  setShowFull(!showFull)
                }}
                className="text-[10px] text-accent hover:text-accent-light transition-colors"
              >
                {isTruncated ? 'Show all' : 'Truncate'}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

interface LlmBodyViewerProps {
  responseText: string | null
  reasoning: string | null
  llmCallId: string
}

export default function LlmBodyViewer({ responseText, reasoning, llmCallId }: LlmBodyViewerProps) {
  const hasContent = responseText || reasoning

  if (!hasContent) {
    return (
      <div className="text-xs text-muted/40 py-2">No response content available</div>
    )
  }

  return (
    <div className="space-y-3 py-3">
      {responseText && (
        <BodySection label="Response" text={responseText} defaultExpanded={true} />
      )}
      {reasoning && (
        <BodySection label="Reasoning" text={reasoning} defaultExpanded={false} />
      )}
      <div className="pt-1">
        <Link
          to={`/llm-calls/${llmCallId}`}
          className="text-accent text-xs hover:text-accent-light transition-colors"
          onClick={(e) => e.stopPropagation()}
        >
          View Full Details &rarr;
        </Link>
      </div>
    </div>
  )
}
