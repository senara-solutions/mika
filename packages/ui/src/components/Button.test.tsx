import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import Button from './Button'

describe('Button — rendered element', () => {
  it('renders a <button> by default, with no `as` prop needed', () => {
    render(<Button onClick={() => {}}>Search</Button>)
    const el = screen.getByRole('button', { name: 'Search' })
    expect(el.tagName).toBe('BUTTON')
    expect(el).toHaveAttribute('type', 'button')
  })

  it('renders an <a href> on the link branch', () => {
    render(
      <Button as="link" href="/getting-started">
        Get Started
      </Button>,
    )
    const el = screen.getByRole('link', { name: 'Get Started' })
    expect(el.tagName).toBe('A')
    expect(el).toHaveAttribute('href', '/getting-started')
  })

  it('an external link carries target and the noopener/noreferrer pair', () => {
    render(
      <Button as="link" href="https://example.com" external>
        Docs
      </Button>,
    )
    const el = screen.getByRole('link', { name: 'Docs' })
    expect(el).toHaveAttribute('target', '_blank')
    expect(el).toHaveAttribute('rel', 'noopener noreferrer')
  })

  it('an internal link carries neither target nor rel', () => {
    render(
      <Button as="link" href="/docs">
        Docs
      </Button>,
    )
    const el = screen.getByRole('link', { name: 'Docs' })
    expect(el).not.toHaveAttribute('target')
    expect(el).not.toHaveAttribute('rel')
  })

  it('honours type="submit" on the action branch', () => {
    render(
      <Button onClick={() => {}} type="submit">
        Save
      </Button>,
    )
    expect(screen.getByRole('button')).toHaveAttribute('type', 'submit')
  })
})

/**
 * These pin the class *mapping* — which variant and size emit which class. What
 * those classes *mean* (the §5 border, colours, the §6 radius, the absence of any
 * colour literal) is pinned in `theme.test.ts`, because the presentation lives in
 * `theme.css`: Tailwind does not scan `packages/ui`, so a utility written here
 * would never be generated. Splitting the two keeps each assertion able to name
 * its own failure.
 */
describe('Button — rulebook §5 variants', () => {
  it.each(['primary', 'secondary', 'tertiary'] as const)(
    'variant %s emits the base class and its own modifier',
    (variant) => {
      render(
        <Button variant={variant} onClick={() => {}}>
          Go
        </Button>,
      )
      const cls = screen.getByRole('button').className
      expect(cls).toContain('mika-btn')
      expect(cls).toContain(`mika-btn--${variant}`)
    },
  )

  it('primary composes the §2 CTA gradient, and no flat background', () => {
    render(<Button onClick={() => {}}>Go</Button>)
    const cls = screen.getByRole('button').className
    // Declared once in theme.css, pinned by theme.test.ts.
    expect(cls).toContain('mika-cta-gradient')
    // The flat `bg-accent` every CTA carried before LC.2.
    expect(cls).not.toContain('bg-accent')
  })

  it('primary is the default variant', () => {
    render(<Button onClick={() => {}}>Go</Button>)
    expect(screen.getByRole('button').className).toContain('mika-btn--primary')
  })

  it.each(['secondary', 'tertiary'] as const)(
    'variant %s does not carry the CTA gradient',
    (variant) => {
      render(
        <Button variant={variant} onClick={() => {}}>
          Go
        </Button>,
      )
      expect(screen.getByRole('button').className).not.toContain('mika-cta-gradient')
    },
  )

  it('the link branch is styled identically to the action branch', () => {
    render(
      <Button as="link" href="/x">
        Go
      </Button>,
    )
    const cls = screen.getByRole('link').className
    expect(cls).toContain('mika-btn')
    expect(cls).toContain('mika-btn--primary')
    expect(cls).toContain('mika-cta-gradient')
  })

  it.each(['sm', 'md', 'lg'] as const)('size %s emits its modifier', (size) => {
    render(
      <Button size={size} onClick={() => {}}>
        Go
      </Button>,
    )
    expect(screen.getByRole('button').className).toContain(`mika-btn--${size}`)
  })

  it('defaults to size md', () => {
    render(<Button onClick={() => {}}>Go</Button>)
    expect(screen.getByRole('button').className).toContain('mika-btn--md')
  })
})

describe('Button — action states', () => {
  it('calls onClick', () => {
    const onClick = vi.fn()
    render(<Button onClick={onClick}>Go</Button>)
    fireEvent.click(screen.getByRole('button'))
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('disabled blocks onClick and sets the DOM disabled attribute', () => {
    const onClick = vi.fn()
    render(
      <Button onClick={onClick} disabled>
        Go
      </Button>,
    )
    const el = screen.getByRole('button')
    expect(el).toBeDisabled()
    fireEvent.click(el)
    expect(onClick).not.toHaveBeenCalled()
  })

  /**
   * D1's explicit decision: `loading` implies `disabled`. Both measured callers
   * (`Traces.tsx`, `SkillVariants.tsx`) drive their disabled state from a
   * mutation's pending flag; leaving the two props independent would allow a
   * loading button that is still clickable — the double submit the state exists
   * to prevent.
   */
  it('loading implies disabled, and blocks onClick', () => {
    const onClick = vi.fn()
    render(
      <Button onClick={onClick} loading>
        Promote
      </Button>,
    )
    const el = screen.getByRole('button')
    expect(el).toBeDisabled()
    fireEvent.click(el)
    expect(onClick).not.toHaveBeenCalled()
  })

  it('loading sets aria-busy', () => {
    render(
      <Button onClick={() => {}} loading>
        Promote
      </Button>,
    )
    expect(screen.getByRole('button')).toHaveAttribute('aria-busy', 'true')
  })

  it('a merely-disabled button is not aria-busy', () => {
    render(
      <Button onClick={() => {}} disabled>
        Promote
      </Button>,
    )
    expect(screen.getByRole('button')).not.toHaveAttribute('aria-busy', 'true')
  })

  it('loading keeps the label, so the button does not change width mid-request', () => {
    render(
      <Button onClick={() => {}} loading>
        Promote
      </Button>,
    )
    expect(screen.getByRole('button')).toHaveTextContent('Promote')
  })

  it('loading swaps the icon for the spinner rather than adding to it', () => {
    const { container } = render(
      <Button onClick={() => {}} loading icon={<svg data-testid="leading-icon" />}>
        Promote
      </Button>,
    )
    expect(screen.queryByTestId('leading-icon')).not.toBeInTheDocument()
    expect(container.querySelectorAll('svg')).toHaveLength(1)
  })

  /**
   * The spinner inside a button must not be a second live region: the button
   * already carries `aria-busy`. Mirror of Spinner.test.tsx's `decorative` case.
   */
  it('the loading spinner is decorative, never a nested live region', () => {
    render(
      <Button onClick={() => {}} loading>
        Promote
      </Button>,
    )
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('renders a leading icon when not loading, hidden from AT', () => {
    render(
      <Button onClick={() => {}} icon={<svg data-testid="leading-icon" />}>
        Search
      </Button>,
    )
    expect(screen.getByTestId('leading-icon')).toBeInTheDocument()
    // Decorative icons must be aria-hidden (packages/ui/CLAUDE.md § Accessibility).
    expect(screen.getByTestId('leading-icon').parentElement).toHaveAttribute(
      'aria-hidden',
      'true',
    )
  })

  it('accepts an explicit aria-label for an icon-only button', () => {
    render(
      <Button onClick={() => {}} ariaLabel="Search traces" icon={<svg />}>
        {''}
      </Button>,
    )
    expect(screen.getByRole('button', { name: 'Search traces' })).toBeInTheDocument()
  })

  it('merges a caller className', () => {
    render(
      <Button onClick={() => {}} className="w-full">
        Go
      </Button>,
    )
    expect(screen.getByRole('button').className).toContain('w-full')
  })
})

describe('Button — accessibility', () => {
  it.each(['primary', 'secondary', 'tertiary'] as const)(
    'has no axe violations (variant %s)',
    async (variant) => {
      const { container } = render(
        <Button variant={variant} onClick={() => {}}>
          Go
        </Button>,
      )
      expect(await axe(container)).toHaveNoViolations()
    },
  )

  it('has no axe violations on the link branch', async () => {
    const { container } = render(
      <Button as="link" href="/x" external>
        Go
      </Button>,
    )
    expect(await axe(container)).toHaveNoViolations()
  })

  it('has no axe violations while loading', async () => {
    const { container } = render(
      <Button onClick={() => {}} loading>
        Promote
      </Button>,
    )
    expect(await axe(container)).toHaveNoViolations()
  })
})
