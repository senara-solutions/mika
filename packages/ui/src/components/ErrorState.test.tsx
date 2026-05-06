import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import ErrorState from './ErrorState'

describe('ErrorState', () => {
  it('renders error alert', () => {
    render(<ErrorState message="Something went wrong" />)
    expect(screen.getByRole('alert')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<ErrorState message="Something went wrong" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations with retry', async () => {
    const { container } = render(
      <ErrorState message="Failed to load" retry={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
