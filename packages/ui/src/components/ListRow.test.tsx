import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import ListRow from './ListRow'

const renderInTable = (ui: React.ReactElement) => {
  return render(<table><tbody>{ui}</tbody></table>)
}

describe('ListRow', () => {
  it('renders static variant', () => {
    renderInTable(<ListRow variant="static"><td>content</td></ListRow>)
    expect(screen.getByText('content')).toBeInTheDocument()
  })

  it('navigable variant responds to keyboard', () => {
    const onClick = vi.fn()
    renderInTable(
      <ListRow variant="navigable" onClick={onClick} ariaLabel="Go to item">
        <td>nav item</td>
      </ListRow>
    )
    const row = screen.getByRole('link')
    fireEvent.keyDown(row, { key: 'Enter' })
    expect(onClick).toHaveBeenCalled()
  })

  it('has no axe violations (static)', async () => {
    const { container } = renderInTable(
      <ListRow variant="static"><td>content</td></ListRow>
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations (navigable)', async () => {
    const { container } = renderInTable(
      <ListRow variant="navigable" onClick={() => {}} ariaLabel="Go to item">
        <td>item</td>
      </ListRow>
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations (expandable)', async () => {
    const { container } = renderInTable(
      <ListRow variant="expandable" isExpanded={false} onToggle={() => {}} ariaLabel="Expand item">
        <td>item</td>
      </ListRow>
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
