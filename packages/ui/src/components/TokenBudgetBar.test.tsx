import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import TokenBudgetBar from './TokenBudgetBar'

describe('TokenBudgetBar', () => {
  // Scenario 1: value=250, max=500 → 50%, green/success color
  it('renders at 50% with success color when below warning threshold', () => {
    render(<TokenBudgetBar value={250} max={500} />)
    const meter = screen.getByRole('meter')
    expect(meter).toHaveAttribute('aria-valuenow', '250')
    expect(meter).toHaveAttribute('aria-valuemax', '500')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-success')
    expect(bar.style.width).toBe('50%')
  })

  // Scenario 2: value=350, max=500 → 70%, warning color
  it('renders at 70% with warning color when between warning and danger thresholds', () => {
    render(<TokenBudgetBar value={350} max={500} />)
    const meter = screen.getByRole('meter')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-warning')
    expect(bar.style.width).toBe('70%')
  })

  // Scenario 3: value=450, max=500 → 90%, error/danger color
  it('renders at 90% with error color when above danger threshold', () => {
    render(<TokenBudgetBar value={450} max={500} />)
    const meter = screen.getByRole('meter')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-error')
    expect(bar.style.width).toBe('90%')
  })

  // Scenario 4: value=0, max=500 → 0%, green color, no visual bar
  it('renders at 0% with success color when value is zero', () => {
    render(<TokenBudgetBar value={0} max={500} />)
    const meter = screen.getByRole('meter')
    expect(meter).toHaveAttribute('aria-valuenow', '0')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-success')
    expect(bar.style.width).toBe('0%')
  })

  // Scenario 5: value=500, max=500 → 100%, error color
  it('renders at 100% with error color when value equals max', () => {
    render(<TokenBudgetBar value={500} max={500} />)
    const meter = screen.getByRole('meter')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-error')
    expect(bar.style.width).toBe('100%')
  })

  // Scenario 6: value=600, max=500 → caps at 100% width, error color
  it('caps at 100% width with error color when value exceeds max', () => {
    render(<TokenBudgetBar value={600} max={500} />)
    const meter = screen.getByRole('meter')
    expect(meter).toHaveAttribute('aria-valuenow', '600')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-error')
    expect(bar.style.width).toBe('100%')
  })

  // Scenario 7: custom thresholds { warning: 0.5, danger: 0.8 } respected
  it('respects custom thresholds', () => {
    // 55% usage — above custom warning (0.5) but below custom danger (0.8)
    render(
      <TokenBudgetBar value={275} max={500} thresholds={{ warning: 0.5, danger: 0.8 }} />
    )
    const meter = screen.getByRole('meter')
    const bar = meter.firstElementChild as HTMLElement
    expect(bar.className).toContain('bg-warning')

    // With default thresholds, 55% would be success (below 0.6)
  })

  // Scenario 8: showFraction=false hides the "250/500" text
  it('hides fraction text when showFraction is false', () => {
    const { container } = render(
      <TokenBudgetBar value={250} max={500} showFraction={false} />
    )
    expect(container.textContent).not.toContain('250/500')
  })

  // Scenario 9: ARIA attributes present with correct values
  it('renders correct ARIA attributes on the meter element', () => {
    render(<TokenBudgetBar value={300} max={500} label="Tokens" />)
    const meter = screen.getByRole('meter')
    expect(meter).toHaveAttribute('aria-valuenow', '300')
    expect(meter).toHaveAttribute('aria-valuemin', '0')
    expect(meter).toHaveAttribute('aria-valuemax', '500')
    expect(meter).toHaveAttribute('aria-label', 'Tokens: 300 of 500')
  })
})
