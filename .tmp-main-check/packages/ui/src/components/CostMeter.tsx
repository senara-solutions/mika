export type CostMeterTier = 'neutral' | 'warning' | 'critical'
export type CostMeterVariant = 'full' | 'chip'

export interface CostMeterProps {
  /** Current cost in USD, expected non-negative. null renders empty state. */
  value: number | null
  /** Warning threshold in USD. Cost ≥ warningAt → amber. */
  warningAt?: number
  /** Critical threshold in USD. Cost ≥ criticalAt → red. */
  criticalAt?: number
  /** 'full' (label + value + threshold indicator) | 'chip' (compact inline). Default: 'full'. */
  variant?: CostMeterVariant
  /** Optional label override (default: 'Cost'). */
  label?: string
  /** ARIA description override for screen readers. */
  ariaLabel?: string
}

const DEFAULT_WARNING = 5
const DEFAULT_CRITICAL = 20

function getTier(
  value: number,
  warningAt: number | undefined,
  criticalAt: number | undefined,
): CostMeterTier {
  if (criticalAt != null && value >= criticalAt) return 'critical'
  if (warningAt != null && value >= warningAt) return 'warning'
  return 'neutral'
}

const TIER_COLORS: Record<CostMeterTier, string> = {
  neutral: 'text-success',
  warning: 'text-warning',
  critical: 'text-error',
}

const TIER_DOT_COLORS: Record<CostMeterTier, string> = {
  neutral: 'bg-success',
  warning: 'bg-warning',
  critical: 'bg-error',
}

function formatUsd(value: number): string {
  if (value < 0.01 && value > 0) return '<$0.01'
  return `$${value.toFixed(2)}`
}

function tierLabel(tier: CostMeterTier): string {
  if (tier === 'critical') return 'critical'
  if (tier === 'warning') return 'warning'
  return 'normal'
}

export default function CostMeter({
  value,
  warningAt = DEFAULT_WARNING,
  criticalAt = DEFAULT_CRITICAL,
  variant = 'full',
  label = 'Cost',
  ariaLabel,
}: CostMeterProps) {
  // Empty state for null/NaN
  if (value == null || Number.isNaN(value)) {
    return (
      <span className="text-muted/30 text-xs font-mono">--</span>
    )
  }

  const tier = getTier(value, warningAt, criticalAt)
  const formattedValue = formatUsd(value)
  const computedAriaLabel =
    ariaLabel ?? `${label} ${formattedValue}, ${tierLabel(tier)}`

  if (variant === 'chip') {
    return (
      <span
        role="status"
        aria-live="polite"
        aria-label={computedAriaLabel}
        className={`inline-flex items-center gap-1.5 ${TIER_COLORS[tier]}`}
      >
        <span
          className={`w-1.5 h-1.5 rounded-full ${TIER_DOT_COLORS[tier]}`}
          aria-hidden="true"
        />
        <span className="text-xs font-mono">{formattedValue}</span>
      </span>
    )
  }

  // Full variant
  return (
    <div
      role="status"
      aria-label={computedAriaLabel}
      className="flex items-center gap-2"
    >
      <span className="text-[10px] text-muted/50 uppercase tracking-wider">
        {label}
      </span>
      <span
        className={`inline-flex items-center gap-1.5 ${TIER_COLORS[tier]}`}
      >
        <span
          className={`w-1.5 h-1.5 rounded-full ${TIER_DOT_COLORS[tier]}`}
          aria-hidden="true"
        />
        <span className="text-sm font-mono font-medium">{formattedValue}</span>
      </span>
    </div>
  )
}
