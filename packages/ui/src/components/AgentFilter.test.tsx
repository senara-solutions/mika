import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import AgentFilter from './AgentFilter'

describe('AgentFilter', () => {
  it('renders without agents', () => {
    const { container } = render(<AgentFilter agents={[]} value="" onChange={() => {}} />)
    expect(container.querySelector('select')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <AgentFilter agents={[{ id: 'a1', name: 'Agent 1' }]} value="" onChange={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
