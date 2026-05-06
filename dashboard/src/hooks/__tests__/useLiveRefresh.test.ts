import { renderHook, act } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { useLiveRefresh } from '../useLiveRefresh'

describe('useLiveRefresh', () => {
  it('returns refetchInterval: false when defaultEnabled is false (default)', () => {
    const { result } = renderHook(() => useLiveRefresh())
    expect(result.current.isLive).toBe(false)
    expect(result.current.refetchInterval).toBe(false)
    expect(result.current.isEffectivelyLive).toBe(false)
  })

  it('returns refetchInterval: 15000 when toggled on and isDefaultView is true', () => {
    const { result } = renderHook(() => useLiveRefresh())
    act(() => result.current.toggle())
    expect(result.current.isLive).toBe(true)
    expect(result.current.refetchInterval).toBe(15_000)
    expect(result.current.isEffectivelyLive).toBe(true)
  })

  it('returns refetchInterval: false when toggled on but isDefaultView is false', () => {
    const { result } = renderHook(() => useLiveRefresh({ isDefaultView: false }))
    act(() => result.current.toggle())
    expect(result.current.isLive).toBe(true)
    expect(result.current.refetchInterval).toBe(false)
  })

  it('toggle() flips isLive state', () => {
    const { result } = renderHook(() => useLiveRefresh())
    expect(result.current.isLive).toBe(false)
    act(() => result.current.toggle())
    expect(result.current.isLive).toBe(true)
    act(() => result.current.toggle())
    expect(result.current.isLive).toBe(false)
  })

  it('isEffectivelyLive is false when isLive is true but isDefaultView is false', () => {
    const { result } = renderHook(() => useLiveRefresh({ isDefaultView: false }))
    act(() => result.current.toggle())
    expect(result.current.isLive).toBe(true)
    expect(result.current.isEffectivelyLive).toBe(false)
  })

  it('returns custom interval when provided and conditions met', () => {
    const { result } = renderHook(() =>
      useLiveRefresh({ defaultEnabled: true, interval: 5_000 })
    )
    expect(result.current.refetchInterval).toBe(5_000)
    expect(result.current.isEffectivelyLive).toBe(true)
  })

  it('isDefaultView defaults to true when not provided', () => {
    const { result } = renderHook(() => useLiveRefresh({ defaultEnabled: true }))
    expect(result.current.isEffectivelyLive).toBe(true)
    expect(result.current.refetchInterval).toBe(15_000)
  })
})
