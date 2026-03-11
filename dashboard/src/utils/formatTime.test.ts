import { describe, it, expect, vi, afterEach } from 'vitest'
import { formatTimestamp, formatRelativeTime } from './formatTime'

describe('formatTimestamp', () => {
  it('formats a unix timestamp', () => {
    const result = formatTimestamp(1700000000)
    expect(result).toContain('Nov')
    expect(result).toContain('14')
  })
})

describe('formatRelativeTime', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('returns "just now" for recent timestamps', () => {
    const now = Date.now() / 1000
    expect(formatRelativeTime(now - 30)).toBe('just now')
  })

  it('returns minutes ago', () => {
    const now = Date.now() / 1000
    expect(formatRelativeTime(now - 180)).toBe('3m ago')
  })

  it('returns hours ago', () => {
    const now = Date.now() / 1000
    expect(formatRelativeTime(now - 7200)).toBe('2h ago')
  })

  it('returns days ago', () => {
    const now = Date.now() / 1000
    expect(formatRelativeTime(now - 172800)).toBe('2d ago')
  })
})
