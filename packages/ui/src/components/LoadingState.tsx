export interface LoadingStateProps {
  variant: 'list' | 'detail'
  rows?: number
  ariaLabel?: string
}

function SkeletonRow() {
  return (
    <div className="flex gap-4 px-4 py-3">
      <div className="h-4 w-1/4 bg-surface-container-high rounded animate-pulse" />
      <div className="h-4 w-1/6 bg-surface-container-high rounded animate-pulse" />
      <div className="h-4 w-1/5 bg-surface-container-high rounded animate-pulse" />
      <div className="h-4 w-1/6 bg-surface-container-high rounded animate-pulse" />
    </div>
  )
}

function ListSkeleton({ rows }: { rows: number }) {
  return (
    <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
      {/* Header row */}
      <div className="flex gap-4 px-4 py-3 border-b border-white/[0.05]">
        <div className="h-3 w-1/5 bg-surface-container-high rounded animate-pulse" />
        <div className="h-3 w-1/6 bg-surface-container-high rounded animate-pulse" />
        <div className="h-3 w-1/5 bg-surface-container-high rounded animate-pulse" />
        <div className="h-3 w-1/6 bg-surface-container-high rounded animate-pulse" />
      </div>
      {/* Data rows */}
      <div className="divide-y divide-white/[0.03]">
        {Array.from({ length: rows }, (_, i) => (
          <SkeletonRow key={i} />
        ))}
      </div>
    </div>
  )
}

function DetailSkeleton() {
  return (
    <div className="space-y-6">
      {/* Metadata strip */}
      <div className="flex items-center gap-3">
        <div className="h-6 w-48 bg-surface-container-high rounded-lg animate-pulse" />
        <div className="h-5 w-20 bg-surface-container-high rounded-full animate-pulse" />
        <div className="h-5 w-16 bg-surface-container-high rounded animate-pulse" />
      </div>
      {/* Main content block */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5 space-y-3">
        <div className="h-4 w-3/4 bg-surface-container-high rounded animate-pulse" />
        <div className="h-4 w-full bg-surface-container-high rounded animate-pulse" />
        <div className="h-4 w-2/3 bg-surface-container-high rounded animate-pulse" />
      </div>
      {/* Sub-section skeleton */}
      <div className="bg-bg-card border border-white/[0.05] rounded-2xl overflow-hidden">
        <div className="px-4 py-3 border-b border-white/[0.05]">
          <div className="h-4 w-32 bg-surface-container-high rounded animate-pulse" />
        </div>
        <div className="divide-y divide-white/[0.03]">
          {Array.from({ length: 3 }, (_, i) => (
            <SkeletonRow key={i} />
          ))}
        </div>
      </div>
    </div>
  )
}

export default function LoadingState({
  variant,
  rows = 6,
  ariaLabel = 'Loading',
}: LoadingStateProps) {
  return (
    <div role="status" aria-live="polite" aria-label={ariaLabel}>
      {variant === 'list' ? (
        <ListSkeleton rows={rows} />
      ) : (
        <DetailSkeleton />
      )}
    </div>
  )
}
