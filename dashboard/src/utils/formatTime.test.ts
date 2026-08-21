import { describe, it, expect, vi, afterEach } from 'vitest'
import { formatTimestamp, formatRelativeTime } from '@samidarko/ui'

describe('formatTimestamp', () => {
  it('formats an ISO 8601 timestamp', () => {
    const result = formatTimestamp('2023-11-14T22:13:20Z')
    expect(result).toContain('Nov')
    expect(result).toContain('14')
  })
})

describe('formatRelativeTime', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('returns "just now" for recent timestamps', () => {
    const now = new Date(Date.now() - 30 * 1000).toISOString()
    expect(formatRelativeTime(now)).toBe('just now')
  })

  it('returns minutes ago', () => {
    const now = new Date(Date.now() - 180 * 1000).toISOString()
    expect(formatRelativeTime(now)).toBe('3m ago')
  })

  it('returns hours ago', () => {
    const now = new Date(Date.now() - 7200 * 1000).toISOString()
    expect(formatRelativeTime(now)).toBe('2h ago')
  })

  it('returns days ago', () => {
    const now = new Date(Date.now() - 172800 * 1000).toISOString()
    expect(formatRelativeTime(now)).toBe('2d ago')
  })
})
