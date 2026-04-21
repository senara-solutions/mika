import { X } from 'lucide-react'
import type { DiffReport } from '../api/variants'

interface Props {
  report: DiffReport
  skillName: string
  provider: string
  model: string
  onClose: () => void
}

const classColors: Record<string, string> = {
  identical: 'text-green-400',
  cosmetic: 'text-blue-400',
  content: 'text-yellow-400',
  structural: 'text-red-400',
}

export default function VariantDiffViewer({ report, skillName, provider, model, onClose }: Props) {
  const lines = report.unified_diff.split('\n')

  return (
    <div className="border border-white/[0.08] rounded-lg bg-bg-card mt-4">
      <div className="flex items-center justify-between px-4 py-3 border-b border-white/[0.05]">
        <div className="flex items-center gap-3">
          <h3 className="text-heading text-sm font-medium">
            Diff: {skillName} &mdash; {provider}/{model}
          </h3>
          <span className={`text-xs font-medium ${classColors[report.classification] ?? 'text-muted'}`}>
            {report.classification}
          </span>
          <span className="text-xs text-muted">
            +{report.added_lines} -{report.removed_lines}
          </span>
        </div>
        <button
          onClick={onClose}
          className="text-muted hover:text-heading transition-colors p-1 rounded hover:bg-white/[0.05]"
        >
          <X size={16} />
        </button>
      </div>

      {report.missing_base_sections.length > 0 && (
        <div className="px-4 py-2 text-xs text-yellow-400 bg-yellow-400/5 border-b border-white/[0.05]">
          Missing base sections: {report.missing_base_sections.join(', ')}
        </div>
      )}

      <div className="overflow-x-auto max-h-96">
        <pre className="text-xs font-mono p-4 leading-relaxed">
          {lines.map((line, i) => {
            let color = 'text-muted'
            if (line.startsWith('+')) color = 'text-green-400'
            else if (line.startsWith('-')) color = 'text-red-400'
            else if (line.startsWith('@@')) color = 'text-blue-400'

            return (
              <div key={i} className={color}>
                {line || '\u00A0'}
              </div>
            )
          })}
        </pre>
      </div>
    </div>
  )
}
