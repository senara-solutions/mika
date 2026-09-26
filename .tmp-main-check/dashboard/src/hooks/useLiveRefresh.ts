import { useState } from 'react'

interface UseLiveRefreshOptions {
  /** Whether auto-refresh starts enabled. Default: false (list pages). Detail pages pass true. */
  defaultEnabled?: boolean
  /** Polling interval in ms. Default: 15_000 (list pages). Detail pages pass 5_000. */
  interval?: number
  /**
   * Opaque boolean gate controlling whether polling is active.
   * Semantics are caller-determined:
   * - List pages: "no filters active AND page === 1" (prevents data-shifting)
   * - Detail pages: "entity is non-terminal" (stops polling when work completes)
   * Default: true
   *
   * Tab visibility: React Query's `refetchIntervalInBackground` defaults to `false`,
   * so polling automatically pauses when the tab is hidden. No custom implementation needed.
   */
  isDefaultView?: boolean
}

interface UseLiveRefreshResult {
  /** Whether the user's toggle is ON */
  isLive: boolean
  /** Toggle the auto-refresh on/off */
  toggle: () => void
  /** The resolved refetchInterval to pass to React Query hooks. false = no polling. */
  refetchInterval: number | false
  /** Whether polling is actually happening (isLive AND isDefaultView) */
  isEffectivelyLive: boolean
}

export function useLiveRefresh(options: UseLiveRefreshOptions = {}): UseLiveRefreshResult {
  const { defaultEnabled = false, interval = 15_000, isDefaultView = true } = options
  const [isLive, setIsLive] = useState(defaultEnabled)

  const isEffectivelyLive = isLive && isDefaultView
  const refetchInterval: number | false = isEffectivelyLive ? interval : false
  const toggle = () => setIsLive(prev => !prev)

  return { isLive, toggle, refetchInterval, isEffectivelyLive }
}
