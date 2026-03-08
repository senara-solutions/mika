import { useState } from 'react'
import { useNavigate } from 'react-router'
import { Search } from 'lucide-react'

export default function Traces() {
  const [traceId, setTraceId] = useState('')
  const navigate = useNavigate()

  function handleSearch() {
    const id = traceId.trim()
    if (id) {
      navigate(`/traces/${id}`)
    }
  }

  return (
    <div>
      <div className="mb-5">
        <h2 className="text-heading text-xl font-semibold">Trace Search</h2>
        <p className="text-sm text-muted/60 mt-1">
          Look up events by trace ID to follow a request across subsystems
        </p>
      </div>

      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-8 max-w-xl">
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted/40" />
            <input
              type="text"
              placeholder="Enter trace ID..."
              value={traceId}
              onChange={(e) => setTraceId(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
              className="w-full bg-bg border border-white/[0.06] rounded-lg pl-9 pr-3 py-2.5 text-sm text-heading placeholder:text-muted/30 focus:outline-none focus:border-accent/40 font-mono"
            />
          </div>
          <button
            onClick={handleSearch}
            disabled={!traceId.trim()}
            className="flex items-center gap-2 px-4 py-2.5 rounded-lg bg-accent text-white text-sm font-medium hover:bg-accent-light transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          >
            <Search size={14} />
            Search
          </button>
        </div>
        <p className="text-xs text-muted/40 mt-3">
          Trace IDs are 32-character hex strings that correlate events across messages, audit log, and tasks.
        </p>
      </div>
    </div>
  )
}
