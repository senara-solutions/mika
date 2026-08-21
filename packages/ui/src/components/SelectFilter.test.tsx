import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import SelectFilter from './SelectFilter'

describe('SelectFilter', () => {
  const options = [
    { value: '', label: 'All' },
    { value: 'a', label: 'Option A' },
    { value: 'b', label: 'Option B' },
  ]

  it('renders a select element with aria-label', () => {
    render(<SelectFilter ariaLabel="Filter by type" value="" onChange={() => {}} options={options} />)
    expect(screen.getByLabelText('Filter by type')).toBeInTheDocument()
  })

  it('renders every option in the dropdown', () => {
    render(<SelectFilter ariaLabel="Filter" value="" onChange={() => {}} options={options} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    expect(select.options).toHaveLength(3)
    expect(select.options[0].textContent).toBe('All')
    expect(select.options[1].textContent).toBe('Option A')
    expect(select.options[2].textContent).toBe('Option B')
  })

  it('reflects the controlled value prop', () => {
    render(<SelectFilter ariaLabel="Filter" value="b" onChange={() => {}} options={options} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    expect(select.value).toBe('b')
  })

  it('calls onChange with the newly selected value', () => {
    const onChange = vi.fn()
    render(<SelectFilter ariaLabel="Filter" value="" onChange={onChange} options={options} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    fireEvent.change(select, { target: { value: 'a' } })
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith('a')
  })

  it('renders no options when the options array is empty', () => {
    render(<SelectFilter ariaLabel="Filter" value="" onChange={() => {}} options={[]} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    expect(select.options).toHaveLength(0)
  })

  it('handles long option labels without crashing', () => {
    const longOptions = [
      { value: '', label: 'All' },
      {
        value: 'x',
        label: 'A very long descriptive label that would show truncation behavior in narrow UI',
      },
    ]
    render(<SelectFilter ariaLabel="Filter" value="x" onChange={() => {}} options={longOptions} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    expect(select.value).toBe('x')
    expect(select.options[1].textContent).toContain('narrow UI')
  })

  it('renders large option lists', () => {
    const many = Array.from({ length: 50 }, (_, i) => ({
      value: `v${i}`,
      label: `Option ${i}`,
    }))
    render(<SelectFilter ariaLabel="Filter" value="v0" onChange={() => {}} options={many} />)
    const select = screen.getByLabelText('Filter') as HTMLSelectElement
    expect(select.options).toHaveLength(50)
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <SelectFilter ariaLabel="Filter by type" value="" onChange={() => {}} options={options} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
