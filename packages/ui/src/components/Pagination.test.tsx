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

  it('has no axe violations', async () => {
    const { container } = render(
      <Pagination page={2} perPage={10} total={100} onPageChange={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
