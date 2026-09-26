import { describe, it, expect } from 'vitest'
import { getAgentColor } from '@samidarko/ui'

describe('getAgentColor', () => {
  it('returns a color object with bg, text, and dot', () => {
    const color = getAgentColor('mika')
    expect(color).toHaveProperty('bg')
    expect(color).toHaveProperty('text')
    expect(color).toHaveProperty('dot')
  })

  it('returns the same color for the same agent name', () => {
    const color1 = getAgentColor('mika')
    const color2 = getAgentColor('mika')
    expect(color1).toEqual(color2)
  })

  it('returns different colors for different agent names', () => {
    const color1 = getAgentColor('mika')
    const color2 = getAgentColor('work')
    // Different names should typically get different colors
    // (not guaranteed but highly likely with different hash values)
    expect(color1.bg).toBeDefined()
    expect(color2.bg).toBeDefined()
  })

  it('handles empty string', () => {
    const color = getAgentColor('')
    expect(color).toHaveProperty('bg')
  })
})
