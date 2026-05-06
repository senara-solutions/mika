import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import CopyButton from './CopyButton'

describe('CopyButton', () => {
  it('renders a button', () => {
    render(<CopyButton text="hello" />)
    expect(screen.getByRole('button')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<CopyButton text="hello" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
