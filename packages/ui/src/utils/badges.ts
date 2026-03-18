export interface EventTypeBadge {
  bg: string
  text: string
  dot: string
  label: string
}

export function eventTypeBadge(type: string): EventTypeBadge {
  switch (type) {
    case 'message':
      return { bg: 'bg-blue-500/15', text: 'text-blue-400', dot: 'bg-blue-400', label: 'Messages' }
    case 'audit':
      return { bg: 'bg-amber-500/15', text: 'text-amber-400', dot: 'bg-amber-400', label: 'Audit Log' }
    case 'task':
      return { bg: 'bg-emerald-500/15', text: 'text-emerald-400', dot: 'bg-emerald-400', label: 'Tasks' }
    default:
      return { bg: 'bg-white/[0.05]', text: 'text-muted', dot: 'bg-muted', label: type }
  }
}

export function eventTypeColor(type: string): string {
  switch (type) {
    case 'message':
      return 'border-blue-400/20 bg-blue-400/5'
    case 'audit':
      return 'border-amber-400/20 bg-amber-400/5'
    case 'task':
      return 'border-emerald-400/20 bg-emerald-400/5'
    default:
      return 'border-white/[0.05] bg-bg-card'
  }
}
