const STATUS_STYLES: Record<string, { bg: string; text: string; dot: string }> = {
  pending: { bg: 'bg-yellow-500/10', text: 'text-yellow-400', dot: 'bg-yellow-400' },
  in_progress: { bg: 'bg-blue-500/10', text: 'text-blue-400', dot: 'bg-blue-400' },
  completed: { bg: 'bg-green-500/10', text: 'text-green-400', dot: 'bg-green-400' },
  failed: { bg: 'bg-red-500/10', text: 'text-red-400', dot: 'bg-red-400' },
  delivered: { bg: 'bg-emerald-500/10', text: 'text-emerald-400', dot: 'bg-emerald-400' },
  blocked: { bg: 'bg-orange-500/10', text: 'text-orange-400', dot: 'bg-orange-400' },
  cancelled: { bg: 'bg-gray-500/10', text: 'text-gray-400', dot: 'bg-gray-400' },
  running: { bg: 'bg-blue-500/10', text: 'text-blue-400', dot: 'bg-blue-400' },
  suspended: { bg: 'bg-amber-500/10', text: 'text-amber-400', dot: 'bg-amber-400' },
  recurring_active: { bg: 'bg-purple-500/10', text: 'text-purple-400', dot: 'bg-purple-400' },
}

const DEFAULT_STYLE = { bg: 'bg-gray-500/10', text: 'text-gray-400', dot: 'bg-gray-400' }

export default function TaskStatusBadge({ status }: { status: string }) {
  const style = STATUS_STYLES[status] ?? DEFAULT_STYLE

  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium ${style.bg} ${style.text}`}
    >
      <span className={`w-1.5 h-1.5 rounded-full ${style.dot}`} />
      {status.replace(/_/g, ' ')}
    </span>
  )
}
