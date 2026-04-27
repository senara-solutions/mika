import { describe, it, expect } from 'vitest'
import { formatApiError } from '@senara-solutions/ui'

describe('formatApiError', () => {
  it('returns connectivity message for TypeError with "fetch"', () => {
    const error = new TypeError('Failed to fetch')
    expect(formatApiError(error)).toBe('Network unreachable. Check your connection.')
  })

  it('falls through for TypeError without "fetch" in message', () => {
    const error = new TypeError('Cannot read properties of null')
    expect(formatApiError(error)).toBe('Cannot read properties of null')
  })

  it('extracts detail from server error envelope', () => {
    const error = { detail: 'Agent not found' }
    expect(formatApiError(error)).toBe('Agent not found')
  })

  it('ignores non-string detail field', () => {
    const error = { detail: 42 }
    expect(formatApiError(error)).toBe('An unexpected error occurred.')
  })

  it('ignores null detail field', () => {
    const error = { detail: null }
    expect(formatApiError(error)).toBe('An unexpected error occurred.')
  })

  it('returns message from Error instance', () => {
    const error = new Error('Something went wrong')
    expect(formatApiError(error)).toBe('Something went wrong')
  })

  it('returns generic fallback for string error', () => {
    expect(formatApiError('some string')).toBe('An unexpected error occurred.')
  })

  it('returns generic fallback for null', () => {
    expect(formatApiError(null)).toBe('An unexpected error occurred.')
  })

  it('returns generic fallback for undefined', () => {
    expect(formatApiError(undefined)).toBe('An unexpected error occurred.')
  })

  it('returns generic fallback for number', () => {
    expect(formatApiError(404)).toBe('An unexpected error occurred.')
  })
})
