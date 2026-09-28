import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import Spinner from './Spinner'

describe('Spinner', () => {
  it('exposes a status live region with a default accessible name', () => {
    render(<Spinner />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Loading')
    expect(status).toHaveAttribute('aria-live', 'polite')
  })

  it('accepts a caller-supplied accessible name', () => {
    render(<Spinner ariaLabel="Promoting variant" />)
    expect(screen.getByRole('status')).toHaveAttribute('aria-label', 'Promoting variant')
  })

  it.each([
    ['xs', 12],
    ['sm', 16],
    ['md', 20],
  ] as const)('renders size %s at %ipx', (size, px) => {
    const { container } = render(<Spinner size={size} />)
    const icon = container.querySelector('svg')
    expect(icon).toHaveAttribute('width', String(px))
    expect(icon).toHaveAttribute('height', String(px))
  })

  it('defaults to size md', () => {
    const { container } = render(<Spinner />)
    expect(container.querySelector('svg')).toHaveAttribute('width', '20')
  })

  it('animates', () => {
    const { container } = render(<Spinner />)
    expect(container.querySelector('svg')?.getAttribute('class')).toContain('mika-spin')
  })

  it('merges a caller className onto the icon', () => {
    const { container } = render(<Spinner className="text-muted" />)
    expect(container.querySelector('svg')?.getAttribute('class')).toContain('text-muted')
  })

  /**
   * `decorative` is what `<Button loading>` passes. Two nested live regions
   * would announce the same fact twice — the button already carries
   * `aria-busy`, so the spinner inside it must be silent. The mirror assertion
   * lives in Button.test.tsx; this one pins the primitive's own half.
   */
  it('drops its live region when decorative, and hides the icon from AT', () => {
    const { container } = render(<Spinner decorative />)
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true')
  })

  it('still renders and animates when decorative', () => {
    const { container } = render(<Spinner decorative size="sm" />)
    const icon = container.querySelector('svg')
    expect(icon).toHaveAttribute('width', '16')
    expect(icon?.getAttribute('class')).toContain('mika-spin')
  })

  it('has no axe violations', async () => {
    const { container } = render(<Spinner />)
    expect(await axe(container)).toHaveNoViolations()
  })

  it('has no axe violations when decorative', async () => {
    const { container } = render(<Spinner decorative />)
    expect(await axe(container)).toHaveNoViolations()
  })
})
