type StatusBadgeVariant = 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'blocked'

interface StatusBadgeProps {
  variant: StatusBadgeVariant
  label: string
  dotPulse?: boolean
}

const VARIANT_STYLES: Record<StatusBadgeVariant, { bg: string; text: string; dot: string }> = {
  success: { bg: 'bg-success/10', text: 'text-success', dot: 'bg-success' },
  warning: { bg: 'bg-warning/10', text: 'text-warning', dot: 'bg-warning' },
  error: { bg: 'bg-error/10', text: 'text-error', dot: 'bg-error' },
  info: { bg: 'bg-accent/10', text: 'text-accent', dot: 'bg-accent' },
  neutral: { bg: 'bg-white/[0.06]', text: 'text-muted', dot: 'bg-muted' },
  blocked: { bg: 'bg-blocked/15', text: 'text-blocked', dot: 'bg-blocked' },
}

export default function StatusBadge({ variant, label, dotPulse = false }: StatusBadgeProps) {
  const style = VARIANT_STYLES[variant]

  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wide ${style.bg} ${style.text}`}
    >
      <span
        className={`w-1.5 h-1.5 rounded-full ${style.dot} ${dotPulse ? 'animate-pulse' : ''}`}
      />
      {label}
    </span>
  )
}

export type { StatusBadgeVariant, StatusBadgeProps }
