import { render, screen, fireEvent, act, waitFor } from '@testing-library/react'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { axe } from 'jest-axe'
import CopyButton from './CopyButton'

describe('CopyButton', () => {
  beforeEach(() => {
    // Mock clipboard API for dynamic state tests
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders a button with aria-label', () => {
    render(<CopyButton text="hello" />)
    const button = screen.getByRole('button')
    expect(button).toBeInTheDocument()
    expect(button).toHaveAttribute('aria-label', 'Copy to clipboard')
  })

  it('updates aria-label to "Copied to clipboard" after copy', async () => {
    render(<CopyButton text="hello" />)
    const button = screen.getByRole('button')

    await act(async () => {
      fireEvent.click(button)
    })

    await waitFor(() => {
      expect(button).toHaveAttribute('aria-label', 'Copied to clipboard')
    })
  })

  it('live region announces "Copied" after copy action', async () => {
    render(<CopyButton text="hello" />)
    const button = screen.getByRole('button')
    const liveRegion = screen.getByRole('status')

    // Initially empty
    expect(liveRegion).toHaveTextContent('')

    await act(async () => {
      fireEvent.click(button)
    })

    await waitFor(() => {
      expect(liveRegion).toHaveTextContent('Copied')
    })
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

  it('has no axe violations in copied state', async () => {
    const { container } = render(<CopyButton text="hello" />)

    await act(async () => {
      fireEvent.click(screen.getByRole('button'))
    })

    await waitFor(async () => {
      const results = await axe(container)
      expect(results).toHaveNoViolations()
    })
  })
})
