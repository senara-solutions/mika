interface StatusBadgeProps {
  active: boolean
}

export default function StatusBadge({ active }: StatusBadgeProps) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium ${
        active
          ? 'bg-emerald-400/10 text-emerald-400'
          : 'bg-amber-400/10 text-amber-400'
      }`}
    >
      <span
        className={`w-1.5 h-1.5 rounded-full ${
          active ? 'bg-emerald-400' : 'bg-amber-400'
        }`}
      />
      {active ? 'Active' : 'Inactive'}
    </span>
  )
}
