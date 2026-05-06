import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import TimeRangeFilter from './TimeRangeFilter'

describe('TimeRangeFilter', () => {
  it('renders preset buttons', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} />)
    expect(screen.getByText('1h')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <TimeRangeFilter value={{}} onChange={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
