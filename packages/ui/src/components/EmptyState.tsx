import type { ReactNode } from 'react'
import { Inbox } from 'lucide-react'

export interface EmptyStateProps {
  message?: string
  title?: string
  icon?: ReactNode
  variant?: 'minimal' | 'card'
}

export default function EmptyState({
  message = 'No data found',
  title,
  icon,
  variant = 'minimal',
}: EmptyStateProps) {
  // undefined = default Inbox icon, null = no icon, ReactNode = custom icon
  const resolvedIcon = icon === null ? null : (icon ?? <Inbox size={32} />)

  const content = (
    <>
      {resolvedIcon && (
        <div className="mb-3">{resolvedIcon}</div>
      )}
      {title && (
        <p className="text-sm font-medium text-heading mb-1">{title}</p>
      )}
      <p className="text-sm">{message}</p>
    </>
  )

  if (variant === 'card') {
    return (
      <div className="flex flex-col items-center justify-center bg-bg-card border border-white/[0.05] rounded-2xl p-8 text-muted/60">
        {content}
      </div>
    )
  }

  return (
    <div className="flex flex-col items-center justify-center py-16 text-muted/60">
      {content}
    </div>
  )
}
