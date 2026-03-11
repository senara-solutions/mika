import { describe, it, expect } from 'vitest'
import { eventTypeBadge, eventTypeColor } from './badges'

describe('eventTypeBadge', () => {
  it('returns blue badge for message type', () => {
    const badge = eventTypeBadge('message')
    expect(badge.label).toBe('Messages')
    expect(badge.bg).toContain('blue')
    expect(badge.text).toContain('blue')
    expect(badge.dot).toContain('blue')
  })

  it('returns amber badge for audit type', () => {
    const badge = eventTypeBadge('audit')
    expect(badge.label).toBe('Audit Log')
    expect(badge.bg).toContain('amber')
  })

  it('returns emerald badge for task type', () => {
    const badge = eventTypeBadge('task')
    expect(badge.label).toBe('Tasks')
    expect(badge.bg).toContain('emerald')
  })

  it('returns default badge for unknown type', () => {
    const badge = eventTypeBadge('unknown')
    expect(badge.label).toBe('unknown')
  })
})

describe('eventTypeColor', () => {
  it('returns blue classes for message', () => {
    expect(eventTypeColor('message')).toContain('blue')
  })

  it('returns amber classes for audit', () => {
    expect(eventTypeColor('audit')).toContain('amber')
  })

  it('returns emerald classes for task', () => {
    expect(eventTypeColor('task')).toContain('emerald')
  })

  it('returns default classes for unknown', () => {
    const result = eventTypeColor('unknown')
    expect(result).toContain('border')
  })
})
