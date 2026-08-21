import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import TaskStatusBadge from './TaskStatusBadge'

/**
 * Rows here mirror the TASK_VARIANT_MAP in TaskStatusBadge.tsx. If the source
 * mapping changes, update this table — the pair is the API contract this
 * adapter exposes to dashboard consumers.
 */
const MAPPED_STATUSES: Array<{
  status: string
  label: string
  colorClass: string
}> = [
  { status: 'pending', label: 'PENDING', colorClass: 'bg-warning/10' },
  { status: 'in_progress', label: 'IN PROGRESS', colorClass: 'bg-accent/10' },
  { status: 'running', label: 'RUNNING', colorClass: 'bg-accent/10' },
  { status: 'completed', label: 'COMPLETED', colorClass: 'bg-success/10' },
  { status: 'delivered', label: 'DELIVERED', colorClass: 'bg-success/10' },
  { status: 'failed', label: 'FAILED', colorClass: 'bg-error/10' },
  { status: 'blocked', label: 'BLOCKED', colorClass: 'bg-blocked/15' },
  { status: 'cancelled', label: 'CANCELLED', colorClass: 'bg-white/[0.06]' },
  { status: 'suspended', label: 'SUSPENDED', colorClass: 'bg-warning/10' },
  { status: 'recurring_active', label: 'RECURRING', colorClass: 'bg-accent/10' },
]

describe('TaskStatusBadge', () => {
  it('renders status text', () => {
    const { container } = render(<TaskStatusBadge status="completed" />)
    expect(container.textContent).toBeTruthy()
  })

  it.each(MAPPED_STATUSES)(
    'maps "$status" → label "$label" with variant class $colorClass',
    ({ status, label, colorClass }) => {
      const { getByText, container } = render(<TaskStatusBadge status={status} />)
      expect(getByText(label)).toBeInTheDocument()
      const wrapper = container.firstElementChild as HTMLElement
      expect(wrapper.className).toContain(colorClass)
    },
  )

  it('falls back to uppercased-underscore-stripped label for unknown status', () => {
    const { getByText } = render(<TaskStatusBadge status="unknown_state" />)
    expect(getByText('UNKNOWN STATE')).toBeInTheDocument()
  })

  it('uses neutral variant for unknown status', () => {
    const { container } = render(<TaskStatusBadge status="mystery" />)
    const wrapper = container.firstElementChild as HTMLElement
    // neutral variant → bg-white/[0.06]
    expect(wrapper.className).toContain('bg-white/[0.06]')
  })

  it('renders empty-string status without crashing (neutral variant, empty label)', () => {
    const { container } = render(<TaskStatusBadge status="" />)
    const wrapper = container.firstElementChild as HTMLElement
    expect(wrapper).not.toBeNull()
    expect(wrapper.className).toContain('bg-white/[0.06]')
  })

  it('has no axe violations', async () => {
    const { container } = render(<TaskStatusBadge status="completed" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
