export interface SelectFilterOption {
  label: string
  value: string
}

export interface SelectFilterProps {
  ariaLabel: string
  value: string
  onChange: (value: string) => void
  options: SelectFilterOption[]
}

export default function SelectFilter({ ariaLabel, value, onChange, options }: SelectFilterProps) {
  return (
    <select
      aria-label={ariaLabel}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40"
    >
      {options.map((opt) => (
        <option key={opt.value} value={opt.value}>
          {opt.label}
        </option>
      ))}
    </select>
  )
}
