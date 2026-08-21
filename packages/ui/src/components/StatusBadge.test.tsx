import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import StatusBadge from './StatusBadge'

const ALL_VARIANTS = ['success', 'warning', 'error', 'info', 'neutral', 'blocked'] as const

const EXPECTED_STYLES: Record<
  (typeof ALL_VARIANTS)[number],
  { bg: string; text: string; dot: string }
> = {
  success: { bg: 'bg-success/10', text: 'text-success', dot: 'bg-success' },
  warning: { bg: 'bg-warning/10', text: 'text-warning', dot: 'bg-warning' },
  error: { bg: 'bg-error/10', text: 'text-error', dot: 'bg-error' },
  info: { bg: 'bg-accent/10', text: 'text-accent', dot: 'bg-accent' },
  neutral: { bg: 'bg-white/[0.06]', text: 'text-muted', dot: 'bg-muted' },
  blocked: { bg: 'bg-blocked/15', text: 'text-blocked', dot: 'bg-blocked' },
}

describe('StatusBadge', () => {
  it('renders with label text', () => {
    const { getByText } = render(<StatusBadge variant="success" label="Active" />)
    expect(getByText('Active')).toBeInTheDocument()
  })

  it.each(ALL_VARIANTS)('applies variant token classes for %s', (variant) => {
    const { container } = render(<StatusBadge variant={variant} label="X" />)
    const wrapper = container.firstElementChild as HTMLElement
    const expected = EXPECTED_STYLES[variant]
    expect(wrapper.className).toContain(expected.bg)
    expect(wrapper.className).toContain(expected.text)
    const dot = wrapper.firstElementChild as HTMLElement
    expect(dot.className).toContain(expected.dot)
  })

  it('applies animate-pulse when dotPulse is true', () => {
    const { container } = render(
      <StatusBadge variant="success" label="Live" dotPulse={true} />,
    )
    const dot = container.querySelector('span > span') as HTMLElement
    expect(dot.className).toContain('animate-pulse')
  })

  it('omits animate-pulse by default', () => {
    const { container } = render(<StatusBadge variant="success" label="Idle" />)
    const dot = container.querySelector('span > span') as HTMLElement
    expect(dot.className).not.toContain('animate-pulse')
  })

  it('renders long label strings without truncation', () => {
    const long = 'a-very-long-status-label-that-tests-overflow-behavior'
    const { getByText } = render(<StatusBadge variant="info" label={long} />)
    expect(getByText(long)).toBeInTheDocument()
  })

  it('renders empty label string without crashing', () => {
    const { container } = render(<StatusBadge variant="neutral" label="" />)
    // Wrapper still renders; only the dot child is present when label is empty.
    expect(container.firstElementChild).not.toBeNull()
    const dot = container.querySelector('span > span') as HTMLElement
    expect(dot).not.toBeNull()
  })

  it.each(ALL_VARIANTS)('has no axe violations (%s variant)', async (variant) => {
    const { container } = render(<StatusBadge variant={variant} label={`${variant} status`} />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
