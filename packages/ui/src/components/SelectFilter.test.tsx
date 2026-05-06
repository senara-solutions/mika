import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import SelectFilter from './SelectFilter'

describe('SelectFilter', () => {
  const options = [
    { value: '', label: 'All' },
    { value: 'a', label: 'Option A' },
    { value: 'b', label: 'Option B' },
  ]

  it('renders a select element with aria-label', () => {
    render(<SelectFilter ariaLabel="Filter by type" value="" onChange={() => {}} options={options} />)
    expect(screen.getByLabelText('Filter by type')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <SelectFilter ariaLabel="Filter by type" value="" onChange={() => {}} options={options} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
