import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import StatusBadge from './StatusBadge'

describe('StatusBadge', () => {
  it('renders with label text', () => {
    const { getByText } = render(<StatusBadge variant="success" label="Active" />)
    expect(getByText('Active')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<StatusBadge variant="success" label="Active" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
