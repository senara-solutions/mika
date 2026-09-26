import StatusBadge from './StatusBadge'
import type { StatusBadgeVariant } from './StatusBadge'

const TASK_VARIANT_MAP: Record<string, { variant: StatusBadgeVariant; label?: string }> = {
  pending: { variant: 'warning', label: 'PENDING' },
  in_progress: { variant: 'info', label: 'IN PROGRESS' },
  running: { variant: 'info', label: 'RUNNING' },
  completed: { variant: 'success', label: 'COMPLETED' },
  delivered: { variant: 'success', label: 'DELIVERED' },
  failed: { variant: 'error', label: 'FAILED' },
  blocked: { variant: 'blocked', label: 'BLOCKED' },
  cancelled: { variant: 'neutral', label: 'CANCELLED' },
  suspended: { variant: 'warning', label: 'SUSPENDED' },
  recurring_active: { variant: 'info', label: 'RECURRING' },
}

const DEFAULT = { variant: 'neutral' as const, label: undefined }

export default function TaskStatusBadge({ status }: { status: string }) {
  const mapped = TASK_VARIANT_MAP[status] ?? DEFAULT
  return <StatusBadge variant={mapped.variant} label={mapped.label ?? status.replace(/_/g, ' ').toUpperCase()} />
}
