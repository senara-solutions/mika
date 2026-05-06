import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import CostMeter from './CostMeter'

describe('CostMeter', () => {
  // Empty state: null value
  it('renders empty state when value is null', () => {
    const { container } = render(<CostMeter value={null} />)
    expect(container.textContent).toBe('--')
    expect(screen.queryByRole('status')).toBeNull()
  })

  // Empty state: NaN value
  it('renders empty state when value is NaN', () => {
    const { container } = render(<CostMeter value={NaN} />)
    expect(container.textContent).toBe('--')
  })

  // Neutral tier: value below warning threshold
  it('renders neutral state when value is below warningAt', () => {
    render(<CostMeter value={2.5} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $2.50, normal')
    expect(status.textContent).toContain('$2.50')
    // Neutral uses success color
    expect(status.innerHTML).toContain('text-success')
  })

  // Warning tier: value at warningAt boundary
  it('renders warning state at warningAt boundary', () => {
    render(<CostMeter value={5} warningAt={5} criticalAt={20} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $5.00, warning')
    expect(status.innerHTML).toContain('text-warning')
  })

  // Warning tier: value between warning and critical
  it('renders warning state between warningAt and criticalAt', () => {
    render(<CostMeter value={12} warningAt={5} criticalAt={20} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $12.00, warning')
    expect(status.innerHTML).toContain('text-warning')
  })

  // Critical tier: value at criticalAt boundary
  it('renders critical state at criticalAt boundary', () => {
    render(<CostMeter value={20} warningAt={5} criticalAt={20} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $20.00, critical')
    expect(status.innerHTML).toContain('text-error')
  })

  // Critical tier: value above critical
  it('renders critical state above criticalAt', () => {
    render(<CostMeter value={50} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $50.00, critical')
    expect(status.innerHTML).toContain('text-error')
  })

  // Chip variant has aria-live polite
  it('chip variant has aria-live polite', () => {
    render(<CostMeter value={3} variant="chip" />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-live', 'polite')
  })

  // Full variant renders label
  it('full variant renders label and value', () => {
    render(<CostMeter value={7.5} />)
    const status = screen.getByRole('status')
    expect(status.textContent).toContain('Cost')
    expect(status.textContent).toContain('$7.50')
  })

  // Custom label override
  it('accepts custom label override', () => {
    render(<CostMeter value={1} label="Run Cost" />)
    const status = screen.getByRole('status')
    expect(status.textContent).toContain('Run Cost')
    expect(status).toHaveAttribute('aria-label', 'Run Cost $1.00, normal')
  })

  // Custom ariaLabel override
  it('accepts custom ariaLabel override', () => {
    render(<CostMeter value={1} ariaLabel="Custom label" />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Custom label')
  })

  // Default thresholds apply when not specified
  it('applies default thresholds ($5 warning, $20 critical)', () => {
    // $3 is below default warning ($5) → neutral
    render(<CostMeter value={3} />)
    const status = screen.getByRole('status')
    expect(status).toHaveAttribute('aria-label', 'Cost $3.00, normal')
    expect(status.innerHTML).toContain('text-success')
  })

  // Zero cost
  it('renders zero cost correctly', () => {
    render(<CostMeter value={0} />)
    const status = screen.getByRole('status')
    expect(status.textContent).toContain('$0.00')
  })

  // Very small cost
  it('renders very small cost as <$0.01', () => {
    render(<CostMeter value={0.001} />)
    const status = screen.getByRole('status')
    expect(status.textContent).toContain('<$0.01')
  })

  // Chip variant shows value without label text
  it('chip variant does not render label text', () => {
    render(<CostMeter value={3.5} variant="chip" />)
    const status = screen.getByRole('status')
    expect(status.textContent).not.toContain('Cost')
    expect(status.textContent).toContain('$3.50')
  })
})
