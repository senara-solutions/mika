import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import ErrorState from './ErrorState'

describe('ErrorState', () => {
  it('renders error alert', () => {
    render(<ErrorState message="Something went wrong" />)
    expect(screen.getByRole('alert')).toBeInTheDocument()
  })

  it('has no axe violations', async () => {
    const { container } = render(<ErrorState message="Something went wrong" />)
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations with retry', async () => {
    const { container } = render(
      <ErrorState message="Failed to load" retry={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('renders detail-section variant with alert role', () => {
    render(<ErrorState message="Section error" variant="detail-section" />)
    expect(screen.getByRole('alert')).toBeInTheDocument()
  })

  it('has no axe violations (detail-section variant)', async () => {
    const { container } = render(
      <ErrorState message="Section error" variant="detail-section" retry={() => {}} />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  // ── mika#1799 D4: the three actions go through <Button>, and these assertions
  // are on what a reader can SEE — the classes `theme.css` paints from, the
  // absence of the §7 violation, the link-safety pair. A test that only checked
  // `getByRole('button')` would have passed on the hand-rolled form too.

  it('paints the list retry with the §2 signature gradient (§5.5 l.272)', () => {
    render(<ErrorState message="Failed to load" retry={() => {}} />)
    const retry = screen.getByRole('button', { name: 'Retry' })
    expect(retry).toHaveClass('mika-btn')
    expect(retry).toHaveClass('mika-btn--primary')
    expect(retry).toHaveClass('mika-cta-gradient')
  })

  it('renders the detail-section retry as the text-only tertiary variant (§5.5 l.238)', () => {
    render(
      <ErrorState message="Section error" variant="detail-section" retry={() => {}} />
    )
    const retry = screen.getByRole('button', { name: 'Retry' })
    expect(retry).toHaveClass('mika-btn--tertiary')
    expect(retry).not.toHaveClass('mika-cta-gradient')
  })

  // §7 "No Pure White" — the hand-rolled gradient button carried `text-white`.
  // Asserted over the whole subtree rather than on the button, because the next
  // regression is as likely to land on a sibling.
  it('carries no pure-white text anywhere (§7)', () => {
    const { container } = render(
      <ErrorState
        message="Failed to load"
        retry={() => {}}
        detailsHref="https://example.test/trace/1"
      />
    )
    expect(container.querySelectorAll('[class*="text-white"]')).toHaveLength(0)
  })

  // `external` emits both halves or neither (Button.tsx:135) — one without the
  // other is the reverse-tabnabbing shape, so both are pinned in one test.
  it('opens the details link with the full noopener/noreferrer pair', () => {
    render(
      <ErrorState message="Failed to load" detailsHref="https://example.test/trace/1" />
    )
    const link = screen.getByRole('link', { name: /View error details/ })
    expect(link).toHaveAttribute('href', 'https://example.test/trace/1')
    expect(link).toHaveAttribute('target', '_blank')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(link).toHaveClass('mika-btn--tertiary')
  })

  it('has no axe violations with both affordances (list variant)', async () => {
    const { container } = render(
      <ErrorState
        message="Failed to load"
        retry={() => {}}
        detailsHref="https://example.test/trace/1"
      />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })

  it('has no axe violations with neither affordance (detail-section variant)', async () => {
    const { container } = render(
      <ErrorState message="Section error" variant="detail-section" />
    )
    const results = await axe(container)
    expect(results).toHaveNoViolations()
  })
})
