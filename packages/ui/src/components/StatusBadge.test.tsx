import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import StatusBadge from './StatusBadge'

const ALL_VARIANTS = ['success', 'warning', 'error', 'info', 'neutral', 'blocked'] as const

describe('StatusBadge', () => {
  it('renders with label text', () => {
    const { getByText } = render(<StatusBadge variant="success" label="Active" />)
    expect(getByText('Active')).toBeInTheDocument()
  })

  it.each(ALL_VARIANTS)('has no axe violations (%s variant)', async (variant) => {
    const { container } = render(<StatusBadge variant={variant} label={`${variant} status`} />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
