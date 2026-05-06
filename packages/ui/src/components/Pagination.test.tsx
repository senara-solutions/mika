import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import Pagination from './Pagination'

describe('Pagination', () => {
  it('renders navigation buttons', () => {
    render(<Pagination page={1} perPage={10} total={100} onPageChange={() => {}} />)
    expect(screen.getByLabelText('Previous page')).toBeInTheDocument()
    expect(screen.getByLabelText('Next page')).toBeInTheDocument()
  })

  it('page indicator has aria-live for screen reader announcements', () => {
    render(<Pagination page={2} perPage={10} total={100} onPageChange={() => {}} />)
    const indicator = screen.getByText(/page 2 of 10/)
    expect(indicator).toHaveAttribute('aria-live', 'polite')
    expect(indicator).toHaveAttribute('aria-atomic', 'true')
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <Pagination page={2} perPage={10} total={100} onPageChange={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
