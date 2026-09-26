import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import TimeRangeFilter from './TimeRangeFilter'

const DEFAULT_PRESET_LABELS = ['15m', '1h', '24h', '7d', '30d']

describe('TimeRangeFilter', () => {
  it('renders preset buttons', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} />)
    expect(screen.getByText('1h')).toBeInTheDocument()
  })

  it.each(DEFAULT_PRESET_LABELS)(
    'renders default preset button "%s"',
    (label) => {
      render(<TimeRangeFilter value={{}} onChange={() => {}} />)
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument()
    },
  )

  it('renders the Custom button alongside the presets', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} />)
    expect(screen.getByRole('button', { name: 'Custom' })).toBeInTheDocument()
  })

  it('clicking a preset emits an ISO 8601 `from` and undefined `to`', () => {
    const onChange = vi.fn()
    render(<TimeRangeFilter value={{}} onChange={onChange} />)
    fireEvent.click(screen.getByRole('button', { name: '1h' }))
    expect(onChange).toHaveBeenCalledTimes(1)
    const arg = onChange.mock.calls[0][0]
    expect(arg.to).toBeUndefined()
    expect(typeof arg.from).toBe('string')
    // ISO 8601 UTC with no ms fraction: YYYY-MM-DDTHH:MM:SSZ
    expect(arg.from).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/)
  })

  it('marks the active preset via aria-pressed after click', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} />)
    const btn = screen.getByRole('button', { name: '24h' })
    expect(btn).toHaveAttribute('aria-pressed', 'false')
    fireEvent.click(btn)
    expect(btn).toHaveAttribute('aria-pressed', 'true')
  })

  it('clicking the active preset again deselects and clears the filter', () => {
    const onChange = vi.fn()
    render(<TimeRangeFilter value={{}} onChange={onChange} />)
    const btn = screen.getByRole('button', { name: '15m' })
    fireEvent.click(btn) // activate
    fireEvent.click(btn) // deselect
    expect(onChange).toHaveBeenLastCalledWith({ from: undefined, to: undefined })
    expect(btn).toHaveAttribute('aria-pressed', 'false')
  })

  it('Custom toggle reveals start/end datetime inputs', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} />)
    expect(screen.queryByLabelText('Start time')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Custom' }))
    expect(screen.getByLabelText('Start time')).toBeInTheDocument()
    expect(screen.getByLabelText('End time')).toBeInTheDocument()
  })

  it('collapsing the Custom panel clears the current range', () => {
    const onChange = vi.fn()
    render(<TimeRangeFilter value={{ from: '2026-01-01T00:00:00Z' }} onChange={onChange} />)
    const custom = screen.getByRole('button', { name: 'Custom' })
    fireEvent.click(custom) // expand
    fireEvent.click(custom) // collapse
    expect(onChange).toHaveBeenLastCalledWith({ from: undefined, to: undefined })
  })

  it('editing the Start time input emits an ISO 8601 `from`', () => {
    const onChange = vi.fn()
    render(<TimeRangeFilter value={{}} onChange={onChange} />)
    fireEvent.click(screen.getByRole('button', { name: 'Custom' }))
    const start = screen.getByLabelText('Start time') as HTMLInputElement
    fireEvent.change(start, { target: { value: '2026-04-01T10:30' } })
    expect(onChange).toHaveBeenCalled()
    const arg = onChange.mock.calls[onChange.mock.calls.length - 1][0]
    expect(typeof arg.from).toBe('string')
    // Precision stripped to seconds — no `.000Z` fragment.
    expect(arg.from).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/)
  })

  it('resetting value externally to {} clears aria-pressed on presets', () => {
    const { rerender } = render(<TimeRangeFilter value={{ from: '2026-01-01T00:00:00Z' }} onChange={() => {}} />)
    // Activate a preset to seed internal state.
    const btn = screen.getByRole('button', { name: '7d' })
    fireEvent.click(btn)
    expect(btn).toHaveAttribute('aria-pressed', 'true')
    // Consumer clears filter externally.
    rerender(<TimeRangeFilter value={{}} onChange={() => {}} />)
    expect(btn).toHaveAttribute('aria-pressed', 'false')
  })

  it('accepts a custom presets prop', () => {
    render(
      <TimeRangeFilter
        value={{}}
        onChange={() => {}}
        presets={[{ label: '5m', durationMs: 5 * 60 * 1000 }]}
      />,
    )
    expect(screen.getByRole('button', { name: '5m' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '1h' })).not.toBeInTheDocument()
  })

  it('renders custom ariaLabel on the wrapper', () => {
    render(<TimeRangeFilter value={{}} onChange={() => {}} ariaLabel="Range" />)
    // The wrapper carries the aria-label as an attribute on a plain div; scope
    // via the presets group we know is inside it.
    const group = screen.getByRole('group', { name: 'Time range presets' })
    const wrapper = group.parentElement as HTMLElement
    expect(wrapper).toHaveAttribute('aria-label', 'Range')
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <TimeRangeFilter value={{}} onChange={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
