export type TokenBudgetTier = 'success' | 'warning' | 'error'

export interface TokenBudgetBarProps {
  value: number
  max: number
  thresholds?: { warning: number; danger: number }
  label?: string
  showFraction?: boolean
}

const DEFAULT_WARNING = 0.6
const DEFAULT_DANGER = 0.85

function getTier(ratio: number, warning: number, danger: number): TokenBudgetTier {
  if (ratio >= danger) return 'error'
  if (ratio >= warning) return 'warning'
  return 'success'
}

const TIER_BAR_COLORS: Record<TokenBudgetTier, string> = {
  success: 'bg-success',
  warning: 'bg-warning',
  error: 'bg-error',
}

export default function TokenBudgetBar({
  value,
  max,
  thresholds,
  label,
  showFraction = true,
}: TokenBudgetBarProps) {
  const warning = thresholds?.warning ?? DEFAULT_WARNING
  const danger = thresholds?.danger ?? DEFAULT_DANGER
  const ratio = max > 0 ? value / max : 0
  const pct = Math.min(100, Math.round(ratio * 100))
  const tier = getTier(ratio, warning, danger)

  return (
    <div className="flex items-center gap-2">
      {label && (
        <span className="text-[10px] text-muted/50 uppercase tracking-wider">
          {label}
        </span>
      )}
      <div
        className="flex-1 h-1 bg-white/[0.06] rounded-full overflow-hidden"
        role="meter"
        aria-valuenow={value}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-label={label ? `${label}: ${value} of ${max}` : `${value} of ${max}`}
      >
        <div
          className={`h-full rounded-full transition-all ${TIER_BAR_COLORS[tier]}`}
          style={{ width: `${pct}%` }}
        />
      </div>
      {showFraction && (
        <span className="text-[10px] text-muted/50 font-mono">
          {value}/{max}
        </span>
      )}
    </div>
  )
}
