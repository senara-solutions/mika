import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import CopyButton from './CopyButton'

describe('CopyButton', () => {
  it('renders a button with aria-label', () => {
    render(<CopyButton text="hello" />)
    const button = screen.getByRole('button')
    expect(button).toBeInTheDocument()
    expect(button).toHaveAttribute('aria-label', 'Copy to clipboard')
  })

  it('has a live region for copy confirmation', () => {
    render(<CopyButton text="hello" />)
    expect(screen.getByRole('status')).toBeInTheDocument()
  })

  it('icons are hidden from screen readers', () => {
    const { container } = render(<CopyButton text="hello" />)
    const icons = container.querySelectorAll('[aria-hidden="true"]')
    expect(icons.length).toBeGreaterThanOrEqual(2)
  })

  it('uses design token color (text-success) not hardcoded color', () => {
    const { container } = render(<CopyButton text="hello" />)
    const checkIcon = container.querySelector('[data-testid="check-icon"]')
    // lucide-react renders SVG elements; className is set via setAttribute
    const cls = checkIcon?.getAttribute('class') ?? ''
    expect(cls).toContain('text-success')
    expect(cls).not.toContain('text-emerald')
  })

  it('has visible focus indicator', () => {
    const { container } = render(<CopyButton text="hello" />)
    const button = container.querySelector('button')
    expect(button?.className).toContain('focus-visible:ring')
  })

  it('has no axe violations', async () => {
    const { container } = render(<CopyButton text="hello" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
