import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import LoadingState from './LoadingState'

describe('LoadingState', () => {
  it('renders with status role', () => {
    render(<LoadingState variant="list" />)
    expect(screen.getByRole('status')).toBeInTheDocument()
  })

  it('has no axe violations (list variant)', async () => {
    const { container } = render(<LoadingState variant="list" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations (detail variant)', async () => {
    const { container } = render(<LoadingState variant="detail" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
