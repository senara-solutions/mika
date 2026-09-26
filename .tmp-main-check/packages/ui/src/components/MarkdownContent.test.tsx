import { render } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import MarkdownContent from './MarkdownContent'

describe('MarkdownContent', () => {
  it('renders markdown content', () => {
    const { container } = render(<MarkdownContent content="**bold text**" />)
    expect(container.querySelector('strong')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<MarkdownContent content="# Heading\n\nA paragraph with a [link](https://example.com)." />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
