import { useState } from 'react'
import { AlertTriangle, FileSearch, Shield, ArrowUpCircle, RefreshCw } from 'lucide-react'
import {
  useVariantsSummary,
  useSkillVariants,
  useVariantDiff,
  useVariantReflect,
  useValidateVariant,
  usePromoteVariant,
} from '../api/variants'
import type { VariantMetadata, ValidationResponse } from '../api/variants'
import VariantDiffViewer from '../components/VariantDiffViewer'

const sourceColors: Record<string, string> = {
  hand_authored: 'text-blue-400',
  generated: 'text-green-400',
  experimental: 'text-yellow-400',
}

const sourceLabels: Record<string, string> = {
  hand_authored: 'hand-authored',
  generated: 'generated',
  experimental: 'experimental',
}

export default function SkillVariants() {
  const [selectedSkill, setSelectedSkill] = useState<string | null>(null)
  const [diffTarget, setDiffTarget] = useState<{ provider: string; model: string } | null>(null)
  const [showReflect, setShowReflect] = useState(false)
  const [validationResult, setValidationResult] = useState<Record<string, ValidationResponse>>({})
  const [confirmPromote, setConfirmPromote] = useState<{ provider: string; model: string } | null>(null)

  const { data: summaries, isLoading: summaryLoading } = useVariantsSummary()
  const { data: variants } = useSkillVariants(selectedSkill)
  const { data: diffReport } = useVariantDiff(
    selectedSkill ?? '',
    diffTarget?.provider ?? '',
    diffTarget?.model ?? '',
    !!selectedSkill && !!diffTarget
  )
  const { data: reflectReport } = useVariantReflect(selectedSkill, showReflect)
  const validateMutation = useValidateVariant()
  const promoteMutation = usePromoteVariant()

  const hasStaleVariants = variants?.some(v => v.stale === true) ?? false

  const handleValidate = async (v: VariantMetadata) => {
    const key = `${v.provider}/${v.model}`
    try {
      const result = await validateMutation.mutateAsync({
        skill: selectedSkill!,
        provider: v.provider,
        model: v.model,
      })
      setValidationResult(prev => ({ ...prev, [key]: result }))
    } catch {
      // Error handled by mutation state
    }
  }

  const handlePromote = async (provider: string, model: string) => {
    try {
      await promoteMutation.mutateAsync({
        skill: selectedSkill!,
        provider,
        model,
      })
      setConfirmPromote(null)
    } catch {
      // Error handled by mutation state
    }
  }

  return (
    <div className="flex h-full">
      {/* Left panel: skill list */}
      <div className="w-64 shrink-0 border-r border-white/[0.05] overflow-y-auto">
        <div className="p-4 border-b border-white/[0.05]">
          <h2 className="text-heading text-sm font-medium">Skills with Variants</h2>
        </div>
        {summaryLoading ? (
          <div className="p-4 text-muted text-sm">Loading...</div>
        ) : !summaries || summaries.length === 0 ? (
          <div className="p-4 text-muted text-sm">No variants found</div>
        ) : (
          <div className="p-2">
            {summaries.map(s => (
              <button
                key={s.skill_name}
                onClick={() => {
                  setSelectedSkill(s.skill_name)
                  setDiffTarget(null)
                  setShowReflect(false)
                  setValidationResult({})
                }}
                className={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors ${
                  selectedSkill === s.skill_name
                    ? 'bg-accent/10 text-accent-light'
                    : 'text-muted hover:text-heading hover:bg-white/[0.03]'
                }`}
              >
                <div className="font-medium">{s.skill_name}</div>
                <div className="flex gap-2 mt-1 text-xs">
                  <span>{s.total} total</span>
                  {s.stale_count > 0 && (
                    <span className="text-yellow-400">{s.stale_count} stale</span>
                  )}
                  {s.experimental > 0 && (
                    <span className="text-yellow-400">{s.experimental} exp</span>
                  )}
                </div>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Right panel: variant table */}
      <div className="flex-1 overflow-y-auto">
        {!selectedSkill ? (
          <div className="flex items-center justify-center h-full text-muted text-sm">
            Select a skill to view its variants
          </div>
        ) : (
          <div className="p-6">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-heading text-lg font-medium">{selectedSkill}</h2>
              <button
                onClick={() => setShowReflect(!showReflect)}
                className="flex items-center gap-2 px-3 py-1.5 text-xs rounded-lg border border-white/[0.1] text-muted hover:text-heading hover:bg-white/[0.03] transition-colors"
              >
                <RefreshCw size={14} />
                {showReflect ? 'Hide Reflect' : 'Reflect'}
              </button>
            </div>

            {/* Stale warning banner */}
            {hasStaleVariants && (
              <div className="flex items-center gap-2 px-4 py-2.5 mb-4 rounded-lg bg-yellow-400/5 border border-yellow-400/20 text-yellow-400 text-sm">
                <AlertTriangle size={16} />
                Base prompt was modified after some variants. Consider re-reviewing stale variants.
              </div>
            )}

            {/* Variant table */}
            {!variants || variants.length === 0 ? (
              <div className="text-muted text-sm">No variants for this skill.</div>
            ) : (
              <div className="border border-white/[0.08] rounded-lg overflow-hidden">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-white/[0.05] bg-white/[0.02]">
                      <th className="text-left px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Provider</th>
                      <th className="text-left px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Model</th>
                      <th className="text-left px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Source</th>
                      <th className="text-right px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Size</th>
                      <th className="text-right px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Lines</th>
                      <th className="text-center px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Stale</th>
                      <th className="text-right px-4 py-2.5 text-muted font-medium text-xs uppercase tracking-wider">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {variants.map(v => {
                      const key = `${v.provider}/${v.model}`
                      const vResult = validationResult[key]
                      return (
                        <tr key={`${v.source}-${key}`} className="border-b border-white/[0.03] hover:bg-white/[0.02]">
                          <td className="px-4 py-2.5 text-heading">{v.provider}</td>
                          <td className="px-4 py-2.5 text-heading font-mono text-xs">{v.model}</td>
                          <td className="px-4 py-2.5">
                            <span className={`text-xs font-medium ${sourceColors[v.source] ?? 'text-muted'}`}>
                              {sourceLabels[v.source] ?? v.source}
                            </span>
                          </td>
                          <td className="px-4 py-2.5 text-right text-muted">{v.size_bytes}B</td>
                          <td className="px-4 py-2.5 text-right text-muted">{v.line_count}</td>
                          <td className="px-4 py-2.5 text-center">
                            {v.stale === true ? (
                              <span className="text-yellow-400 text-xs">yes</span>
                            ) : v.stale === false ? (
                              <span className="text-green-400 text-xs">no</span>
                            ) : (
                              <span className="text-muted text-xs">-</span>
                            )}
                          </td>
                          <td className="px-4 py-2.5 text-right">
                            <div className="flex items-center justify-end gap-1">
                              <button
                                onClick={() => setDiffTarget({ provider: v.provider, model: v.model })}
                                title="View diff"
                                className="p-1 text-muted hover:text-heading transition-colors"
                              >
                                <FileSearch size={14} />
                              </button>
                              <button
                                onClick={() => handleValidate(v)}
                                title="Validate"
                                className="p-1 text-muted hover:text-heading transition-colors"
                              >
                                <Shield size={14} />
                              </button>
                              {v.source === 'experimental' && (
                                <button
                                  onClick={() => setConfirmPromote({ provider: v.provider, model: v.model })}
                                  title="Promote to generated"
                                  className="p-1 text-muted hover:text-green-400 transition-colors"
                                >
                                  <ArrowUpCircle size={14} />
                                </button>
                              )}
                            </div>
                            {vResult && (
                              <div className="mt-1 text-xs text-left">
                                {vResult.results.map((r, i) => (
                                  <div key={i} className={
                                    r.level === 'ok' ? 'text-green-400' :
                                    r.level === 'warn' ? 'text-yellow-400' : 'text-red-400'
                                  }>
                                    [{r.level.toUpperCase()}] {r.message}
                                  </div>
                                ))}
                              </div>
                            )}
                          </td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              </div>
            )}

            {/* Diff viewer */}
            {diffTarget && diffReport && (
              <VariantDiffViewer
                report={diffReport}
                skillName={selectedSkill}
                provider={diffTarget.provider}
                model={diffTarget.model}
                onClose={() => setDiffTarget(null)}
              />
            )}

            {/* Reflection report */}
            {showReflect && reflectReport && (
              <div className="mt-4 border border-white/[0.08] rounded-lg bg-bg-card p-4">
                <h3 className="text-heading text-sm font-medium mb-3">Reflection Report</h3>
                <div className="text-xs text-muted mb-3">
                  Base last modified: {reflectReport.base_last_modified ?? 'unknown'}
                </div>
                {reflectReport.impacts.length === 0 ? (
                  <div className="text-muted text-sm">No production variants to reflect on.</div>
                ) : (
                  <div className="space-y-3">
                    {reflectReport.impacts.map((impact, i) => (
                      <div key={i} className="border border-white/[0.05] rounded-lg p-3">
                        <div className="flex items-center gap-2">
                          <span className="text-heading text-sm">
                            {impact.metadata.provider}/{impact.metadata.model}
                          </span>
                          <span className={`text-xs ${sourceColors[impact.metadata.source] ?? 'text-muted'}`}>
                            {sourceLabels[impact.metadata.source] ?? impact.metadata.source}
                          </span>
                          {impact.metadata.stale === true && (
                            <span className="text-yellow-400 text-xs">[STALE]</span>
                          )}
                        </div>
                        <div className="mt-1 text-xs text-muted">
                          {impact.diff_summary.classification} &mdash; +{impact.diff_summary.added_lines} -{impact.diff_summary.removed_lines} lines
                        </div>
                        {impact.diff_summary.missing_sections.length > 0 && (
                          <div className="mt-1 text-xs text-yellow-400">
                            Missing: {impact.diff_summary.missing_sections.join(', ')}
                          </div>
                        )}
                        <div className="mt-1 text-xs text-accent-light">{impact.recommendation}</div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Promote confirmation modal */}
            {confirmPromote && (
              <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
                <div className="bg-bg-card border border-white/[0.1] rounded-xl p-6 max-w-md w-full mx-4">
                  <h3 className="text-heading font-medium mb-2">Promote Variant</h3>
                  <p className="text-muted text-sm mb-4">
                    Promote <span className="text-heading">{confirmPromote.provider}/{confirmPromote.model}</span> from
                    experimental to generated? This will run validation first.
                  </p>
                  {promoteMutation.isError && (
                    <div className="text-red-400 text-sm mb-4 p-2 bg-red-400/5 rounded">
                      {promoteMutation.error.message}
                    </div>
                  )}
                  <div className="flex justify-end gap-3">
                    <button
                      onClick={() => { setConfirmPromote(null); promoteMutation.reset() }}
                      className="px-4 py-2 text-sm text-muted hover:text-heading transition-colors"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={() => handlePromote(confirmPromote.provider, confirmPromote.model)}
                      disabled={promoteMutation.isPending}
                      className="px-4 py-2 text-sm bg-green-600 text-white rounded-lg hover:bg-green-500 transition-colors disabled:opacity-50"
                    >
                      {promoteMutation.isPending ? 'Promoting...' : 'Promote'}
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
