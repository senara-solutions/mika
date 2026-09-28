import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import AuthCard from './AuthCard'

describe('AuthCard — header grammar', () => {
  it('renders the title as the page h1', () => {
    render(<AuthCard title="Welcome back">form</AuthCard>)
    const heading = screen.getByRole('heading', { name: 'Welcome back' })
    expect(heading.tagName).toBe('H1')
  })

  it('renders the subtitle when given', () => {
    render(
      <AuthCard title="Welcome back" subtitle="Sign in to continue">
        form
      </AuthCard>,
    )
    expect(screen.getByText('Sign in to continue')).toBeInTheDocument()
  })

  it('renders no subtitle node when none is given', () => {
    const { container } = render(<AuthCard title="Welcome back">form</AuthCard>)
    expect(container.querySelector('.mika-auth-card__subtitle')).toBeNull()
  })

  /**
   * A slot, not an import — the library cannot depend on a consumer's own
   * components. Same separation `<AgentFilter agents={...}>` keeps.
   */
  it('renders a consumer-injected logo', () => {
    render(
      <AuthCard title="Welcome back" logo={<svg data-testid="logo" />}>
        form
      </AuthCard>,
    )
    expect(screen.getByTestId('logo')).toBeInTheDocument()
  })

  it('renders no logo slot when none is given', () => {
    const { container } = render(<AuthCard title="Welcome back">form</AuthCard>)
    expect(container.querySelector('.mika-auth-card__logo')).toBeNull()
  })
})

describe('AuthCard — content and layout', () => {
  it('renders free children below the header', () => {
    render(
      <AuthCard title="Welcome back">
        <button type="button">Continue with Google</button>
        <a href="/signup">Sign up</a>
      </AuthCard>,
    )
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Sign up' })).toBeInTheDocument()
  })

  /**
   * Both measured callers write the same full-height centring around the card,
   * so the primitive owns it — that is the "centred" half of the name.
   */
  it('owns the full-height centring, not just the card', () => {
    const { container } = render(<AuthCard title="Welcome back">form</AuthCard>)
    const screenEl = container.querySelector('.mika-auth-screen')
    expect(screenEl).not.toBeNull()
    expect(screenEl?.querySelector('.mika-auth-card')).not.toBeNull()
  })

  it('merges a caller className onto the card, not the screen', () => {
    const { container } = render(
      <AuthCard title="Welcome back" className="gap-6">
        form
      </AuthCard>,
    )
    expect(container.querySelector('.mika-auth-card')?.className).toContain('gap-6')
    expect(container.querySelector('.mika-auth-screen')?.className).not.toContain('gap-6')
  })

  it('leaves no trailing space in the card class list when no className is given', () => {
    const { container } = render(<AuthCard title="Welcome back">form</AuthCard>)
    expect(container.querySelector('.mika-auth-card')?.className).toBe('mika-auth-card')
  })
})

describe('AuthCard — accessibility', () => {
  it('has no axe violations in the fullest measured shape', async () => {
    const { container } = render(
      <AuthCard
        title="Welcome back"
        subtitle="Sign in to continue"
        logo={<svg data-testid="logo" aria-hidden="true" />}
      >
        <button type="button">Continue with Google</button>
        <a href="/signup">Sign up</a>
      </AuthCard>,
    )
    expect(await axe(container)).toHaveNoViolations()
  })

  it('has no axe violations in the minimal shape', async () => {
    const { container } = render(<AuthCard title="Welcome back">form</AuthCard>)
    expect(await axe(container)).toHaveNoViolations()
  })
})
