import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import AgentFilter from './AgentFilter'

describe('AgentFilter', () => {
  const agents = [
    { id: 'a1', name: 'Agent One' },
    { id: 'a2', name: 'Agent Two' },
  ]

  it('renders without agents', () => {
    const { container } = render(<AgentFilter agents={[]} value="" onChange={() => {}} />)
    expect(container.querySelector('select')).toBeInTheDocument()
  })

  it('renders default "All Agents" as the first option', () => {
    render(<AgentFilter agents={agents} value="" onChange={() => {}} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    expect(select.options[0].textContent).toBe('All Agents')
    expect(select.options[0].value).toBe('')
  })

  it('renders a custom emptyLabel when provided', () => {
    render(
      <AgentFilter agents={agents} value="" onChange={() => {}} emptyLabel="Any agent" />,
    )
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    expect(select.options[0].textContent).toBe('Any agent')
  })

  it('renders each agent as an option with id and name', () => {
    render(<AgentFilter agents={agents} value="" onChange={() => {}} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    // 1 empty + 2 agents = 3 total
    expect(select.options).toHaveLength(3)
    expect(select.options[1].value).toBe('a1')
    expect(select.options[1].textContent).toBe('Agent One')
    expect(select.options[2].value).toBe('a2')
    expect(select.options[2].textContent).toBe('Agent Two')
  })

  it('handles undefined agents (falls back to only the empty option)', () => {
    render(<AgentFilter agents={undefined} value="" onChange={() => {}} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    expect(select.options).toHaveLength(1)
    expect(select.options[0].value).toBe('')
  })

  it('calls onChange with the selected agent id', () => {
    const onChange = vi.fn()
    render(<AgentFilter agents={agents} value="" onChange={onChange} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    fireEvent.change(select, { target: { value: 'a2' } })
    expect(onChange).toHaveBeenCalledWith('a2')
  })

  it('reflects the controlled value prop', () => {
    render(<AgentFilter agents={agents} value="a1" onChange={() => {}} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    expect(select.value).toBe('a1')
  })

  it('handles long agent names without crashing', () => {
    const longNameAgents = [
      { id: 'long', name: 'agent-with-a-very-long-name-that-tests-overflow-in-narrow-filter-bars' },
    ]
    render(<AgentFilter agents={longNameAgents} value="" onChange={() => {}} />)
    const select = screen.getByLabelText('Filter by agent') as HTMLSelectElement
    expect(select.options[1].textContent).toContain('overflow')
  })

  it('has no axe violations', async () => {
    const { container } = render(
      <AgentFilter agents={agents} value="" onChange={() => {}} />,
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
