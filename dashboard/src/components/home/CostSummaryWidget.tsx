import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../../api/client.ts'
import type { CostTrendResponse } from '../../api/llmCalls.ts'
import CostTrendChart, { type ChartVariant } from '../CostTrendChart.tsx'
import WidgetSection from './WidgetSection.tsx'

interface CostSummaryWidgetProps {
  refetchInterval?: number | false
}

export default function CostSummaryWidget({ refetchInterval }: CostSummaryWidgetProps) {
  // Lock to 'total' variant for the compact landing page view
  const [variant] = useState<ChartVariant>('total')
  const { data, isLoading, error, refetch } = useQuery<CostTrendResponse>({
    queryKey: ['cost-trend', {}],
    queryFn: () => apiFetch('/llm-calls/cost-trend', {}),
    refetchInterval,
  })

  return (
    <WidgetSection
      label="Cost (24h)"
      viewAllTo="/llm-calls"
    >
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
    </WidgetSection>
  )
}

export type { CostSummaryWidgetProps }
