import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../../api/client.ts'
import type { CostTrendResponse } from '../../api/llmCalls.ts'
import type { ChartVariant } from '../CostTrendChart.tsx'
import CostTrendChart from '../CostTrendChart.tsx'
import { ArrowRight } from 'lucide-react'

interface CostSummaryWidgetProps {
  refetchInterval?: number | false
}

/**
 * Cost summary widget for the landing page.
 * Uses CostTrendChart directly (which has its own bg-bg-card container)
 * instead of wrapping in WidgetSection to avoid double-card nesting.
 */
export default function CostSummaryWidget({ refetchInterval }: CostSummaryWidgetProps) {
  const variant: ChartVariant = 'total'
  const { data, isLoading, error, refetch } = useQuery<CostTrendResponse>({
    queryKey: ['cost-trend', {}],
    queryFn: () => apiFetch('/llm-calls/cost-trend', {}),
    refetchInterval,
  })

  return (
    <section>
      {/* Section header matching WidgetSection style */}
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-[11px] text-muted/60 font-medium uppercase tracking-wider">
          Cost (24h)
        </h3>
        <Link
          to="/llm-calls"
          className="flex items-center gap-1 text-[11px] text-muted/50 hover:text-accent-light transition-colors"
        >
          View all
          <ArrowRight size={12} />
        </Link>
      </div>
      {/* CostTrendChart provides its own bg-bg-card container with loading/error/empty states */}
      <CostTrendChart
        data={data?.buckets}
        variant={variant}
        onVariantChange={() => {}}
        bucketSize={data?.bucket_size ?? 'hour'}
        isLoading={isLoading}
        error={error}
        onRetry={() => refetch()}
        hasEstimatedPricing={data?.has_estimated_pricing}
        defaultRange="last 24h"
      />
    </section>
  )
}
