import { Inbox } from 'lucide-react'

interface EmptyStateProps {
  message?: string
}

export default function EmptyState({ message = 'No data found' }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center py-16 text-muted/60">
      <Inbox size={32} className="mb-3" />
      <p className="text-sm">{message}</p>
    </div>
  )
}
