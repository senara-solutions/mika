import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import EmptyState from './EmptyState'

describe('EmptyState', () => {
  it('renders default empty state', () => {
    const { container } = render(<EmptyState />)
    expect(container.textContent).toBeTruthy()
  })

  it('renders with custom message', () => {
    const { getByText } = render(<EmptyState message="No items found" />)
    expect(getByText('No items found')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<EmptyState message="No results" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations with action', async () => {
    const { container } = render(
      <EmptyState message="No results" action={{ label: 'Clear filters', onClick: () => {} }} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
