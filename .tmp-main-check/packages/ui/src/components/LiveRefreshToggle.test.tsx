import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import LiveRefreshToggle from './LiveRefreshToggle'

describe('LiveRefreshToggle', () => {
  it('renders LIVE badge when isLive is true', () => {
    render(<LiveRefreshToggle isLive={true} onToggle={() => {}} />)
    expect(screen.getByText('Live')).toBeInTheDocument()
  })

  it('hides LIVE badge when isLive is false', () => {
    render(<LiveRefreshToggle isLive={false} onToggle={() => {}} />)
    expect(screen.queryByText('Live')).not.toBeInTheDocument()
  })

  it('toggle switch reflects aria-checked matching isLive prop (true)', () => {
    render(<LiveRefreshToggle isLive={true} onToggle={() => {}} />)
    const toggle = screen.getByRole('switch')
    expect(toggle).toHaveAttribute('aria-checked', 'true')
  })

  it('toggle switch reflects aria-checked matching isLive prop (false)', () => {
    render(<LiveRefreshToggle isLive={false} onToggle={() => {}} />)
    const toggle = screen.getByRole('switch')
    expect(toggle).toHaveAttribute('aria-checked', 'false')
  })

  it('calls onToggle when switch is clicked', () => {
    const onToggle = vi.fn()
    render(<LiveRefreshToggle isLive={false} onToggle={onToggle} />)
    fireEvent.click(screen.getByRole('switch'))
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  it('disabled prop prevents click and applies muted styling', () => {
    const onToggle = vi.fn()
    const { container } = render(
      <LiveRefreshToggle isLive={true} onToggle={onToggle} disabled />
    )
    fireEvent.click(screen.getByRole('switch'))
    expect(onToggle).not.toHaveBeenCalled()
    const wrapper = container.firstElementChild as HTMLElement
    expect(wrapper.className).toContain('opacity-50')
    expect(wrapper.className).toContain('cursor-not-allowed')
  })

  it('has role="switch" and aria-checked attributes for accessibility', () => {
    render(<LiveRefreshToggle isLive={true} onToggle={() => {}} />)
    const toggle = screen.getByRole('switch')
    expect(toggle).toBeInTheDocument()
    expect(toggle).toHaveAttribute('aria-checked')
  })

  it('has no axe violations', async () => {
    const { container } = render(<LiveRefreshToggle isLive={true} onToggle={() => {}} />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
