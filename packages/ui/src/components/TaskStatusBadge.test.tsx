import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import TaskStatusBadge from './TaskStatusBadge'

describe('TaskStatusBadge', () => {
  it('renders status text', () => {
    const { container } = render(<TaskStatusBadge status="completed" />)
    expect(container.textContent).toBeTruthy()
  })

  it('has no axe violations', async () => {
    const { container } = render(<TaskStatusBadge status="completed" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
