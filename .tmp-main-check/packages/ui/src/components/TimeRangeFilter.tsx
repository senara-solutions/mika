import { useState, useCallback, useMemo, useEffect } from 'react'

/**
 * Time range for filtering observability data.
 * `from` and `to` are ISO 8601 UTC strings (e.g., "2026-04-26T22:00:00.000Z").
 * `undefined` means no bound in that direction.
 */
export interface TimeRange {
  from?: string
  to?: string
}

export interface TimeRangePreset {
  label: string
  durationMs: number
}

export interface TimeRangeFilterProps {
  value: TimeRange
  onChange: (range: TimeRange) => void
  presets?: TimeRangePreset[]
  ariaLabel?: string
}

const DEFAULT_PRESETS: TimeRangePreset[] = [
  { label: '15m', durationMs: 15 * 60 * 1000 },
  { label: '1h', durationMs: 60 * 60 * 1000 },
  { label: '24h', durationMs: 24 * 60 * 60 * 1000 },
  { label: '7d', durationMs: 7 * 24 * 60 * 60 * 1000 },
  { label: '30d', durationMs: 30 * 24 * 60 * 60 * 1000 },
]

/**
 * `<TimeRangeFilter />` emits ISO 8601 UTC strings via `new Date(localInputValue).toISOString()`.
 * This interprets the user's datetime-local input as the browser's local timezone.
 *
 * Known limitation: users whose browser timezone differs from their operational timezone
 * see results offset by the difference. Multi-timezone support is a follow-up trigger;
 * see luminescent-core.md section 5.4 for the rulebook clause.
 */
export default function TimeRangeFilter({
  value,
  onChange,
  presets = DEFAULT_PRESETS,
  ariaLabel = 'Time range filter',
}: TimeRangeFilterProps) {
  const [showCustom, setShowCustom] = useState(false)
  const [activePreset, setActivePreset] = useState<string | null>(null)

  // Reset visual state when the filter is cleared externally (e.g., "Clear All" button)
  useEffect(() => {
    if (!value.from && !value.to) {
      setActivePreset(null)
      setShowCustom(false)
    }
  }, [value.from, value.to])

  const handlePresetClick = useCallback(
    (preset: TimeRangePreset) => {
      if (activePreset === preset.label) {
        // Deselect: clear the filter
        setActivePreset(null)
        onChange({ from: undefined, to: undefined })
        return
      }
      setActivePreset(preset.label)
      setShowCustom(false)
      const from = stripMilliseconds(new Date(Date.now() - preset.durationMs).toISOString())
      onChange({ from, to: undefined })
    },
    [activePreset, onChange],
  )

  const handleCustomToggle = useCallback(() => {
    const next = !showCustom
    setShowCustom(next)
    if (next) {
      setActivePreset(null)
    } else {
      // Clear custom range when collapsing
      onChange({ from: undefined, to: undefined })
    }
  }, [showCustom, onChange])

  const handleCustomFrom = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const localValue = e.target.value
      const from = safeToIsoString(localValue)
      onChange({ from, to: value.to })
    },
    [onChange, value.to],
  )

  const handleCustomTo = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const localValue = e.target.value
      const to = safeToIsoString(localValue)
      onChange({ from: value.from, to })
    },
    [onChange, value.from],
  )

  // Convert ISO string back to datetime-local format for the input value
  const fromLocalValue = useMemo(() => {
    if (!value.from) return ''
    const d = new Date(value.from)
    return toDatetimeLocalString(d)
  }, [value.from])

  const toLocalValue = useMemo(() => {
    if (!value.to) return ''
    const d = new Date(value.to)
    return toDatetimeLocalString(d)
  }, [value.to])

  return (
    <div className="flex items-center gap-2 flex-wrap" aria-label={ariaLabel}>
      <div
        className="flex items-center gap-1"
        role="group"
        aria-label="Time range presets"
      >
        {presets.map((preset) => (
          <button
            key={preset.label}
            type="button"
            aria-pressed={activePreset === preset.label}
            onClick={() => handlePresetClick(preset)}
            className={`px-2.5 py-1 text-xs font-medium rounded-lg transition-colors ${
              activePreset === preset.label
                ? 'bg-[var(--color-primary,#ada3ff)]/20 text-[var(--color-primary,#ada3ff)] ring-1 ring-[var(--color-primary,#ada3ff)]/40'
                : 'text-[var(--color-on-surface-variant,#aaabaf)] hover:text-[var(--color-on-surface,#e8e8ec)] hover:bg-[var(--color-surface-container-high,#1d2024)]'
            }`}
          >
            {preset.label}
          </button>
        ))}
        <button
          type="button"
          aria-pressed={showCustom}
          onClick={handleCustomToggle}
          className={`px-2.5 py-1 text-xs font-medium rounded-lg transition-colors ${
            showCustom
              ? 'bg-[var(--color-primary,#ada3ff)]/20 text-[var(--color-primary,#ada3ff)] ring-1 ring-[var(--color-primary,#ada3ff)]/40'
              : 'text-[var(--color-on-surface-variant,#aaabaf)] hover:text-[var(--color-on-surface,#e8e8ec)] hover:bg-[var(--color-surface-container-high,#1d2024)]'
          }`}
        >
          Custom
        </button>
      </div>

      {showCustom && (
        <div className="flex items-center gap-2">
          <input
            type="datetime-local"
            aria-label="Start time"
            value={fromLocalValue}
            onChange={handleCustomFrom}
            className="px-2 py-1 text-xs rounded-lg bg-[var(--color-surface-container-lowest,#000)] text-[var(--color-on-surface,#e8e8ec)] border border-[var(--color-outline-variant,#46484b)]/20 focus:border-[var(--color-primary,#ada3ff)] focus:outline-none"
          />
          <span className="text-xs text-[var(--color-on-surface-variant,#aaabaf)]">
            to
          </span>
          <input
            type="datetime-local"
            aria-label="End time"
            value={toLocalValue}
            onChange={handleCustomTo}
            className="px-2 py-1 text-xs rounded-lg bg-[var(--color-surface-container-lowest,#000)] text-[var(--color-on-surface,#e8e8ec)] border border-[var(--color-outline-variant,#46484b)]/20 focus:border-[var(--color-primary,#ada3ff)] focus:outline-none"
          />
        </div>
      )}
    </div>
  )
}

/** Convert a Date to the `YYYY-MM-DDTHH:mm` format required by datetime-local inputs. */
function toDatetimeLocalString(d: Date): string {
  const year = d.getFullYear()
  const month = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  const hours = String(d.getHours()).padStart(2, '0')
  const minutes = String(d.getMinutes()).padStart(2, '0')
  return `${year}-${month}-${day}T${hours}:${minutes}`
}

/**
 * Parse a datetime-local string to ISO 8601 UTC, returning undefined on invalid input.
 * Strips milliseconds to match backend TEXT timestamp format (%Y-%m-%dT%H:%M:%SZ)
 * and ensure correct lexicographic comparison at the `to` boundary.
 */
function safeToIsoString(localValue: string): string | undefined {
  if (!localValue) return undefined
  const d = new Date(localValue)
  if (isNaN(d.getTime())) return undefined
  return stripMilliseconds(d.toISOString())
}

/** Strip `.000Z` suffix, replacing with `Z` to match backend timestamp precision. */
function stripMilliseconds(iso: string): string {
  return iso.replace(/\.\d{3}Z$/, 'Z')
}
