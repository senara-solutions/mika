import StatusBadge from './StatusBadge.tsx'

interface LiveRefreshToggleProps {
  isLive: boolean
  onToggle: () => void
  disabled?: boolean
  className?: string
}

export default function LiveRefreshToggle({
  isLive,
  onToggle,
  disabled = false,
  className = '',
}: LiveRefreshToggleProps) {
  return (
    <div
      className={`flex items-center gap-3 ${disabled ? 'opacity-50 cursor-not-allowed' : ''} ${className}`}
    >
      {isLive && <StatusBadge variant="success" label="Live" dotPulse />}
      <label className="flex items-center gap-2 text-xs text-muted cursor-pointer">
        Auto-refresh
        <button
          role="switch"
          aria-checked={isLive}
          onClick={disabled ? undefined : onToggle}
          disabled={disabled}
          className={`relative w-9 h-5 rounded-full transition-colors ${
            isLive ? 'bg-accent' : 'bg-white/[0.1]'
          } ${disabled ? 'cursor-not-allowed' : ''}`}
        >
          <span
            className={`absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
              isLive ? 'left-[18px]' : 'left-0.5'
            }`}
          />
        </button>
      </label>
    </div>
  )
}

export type { LiveRefreshToggleProps }
