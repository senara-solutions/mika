import { render, screen } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { axe } from 'jest-axe'
import Stepper from './Stepper'

/**
 * The two tier shapes the measured caller produces, with its terminal `success`
 * step already filtered out of the display — which is where the caller filters
 * it (`steps.filter(s => s !== 'success')`).
 */
const BYO_KEY_TIER = [
  { id: 'telegram', label: 'Connect Telegram' },
  { id: 'apikey', label: 'Add API key' },
  { id: 'provisioning', label: 'Provisioning' },
]

const MANAGED_TIER = [
  { id: 'telegram', label: 'Connect Telegram' },
  { id: 'provisioning', label: 'Provisioning' },
]

const pills = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('.mika-stepper__pill')).map((el) => el.textContent)

const connectors = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('.mika-stepper__connector'))

describe('Stepper — step states', () => {
  it('renders one item per step, labelled', () => {
    render(<Stepper steps={BYO_KEY_TIER} current="telegram" />)
    expect(screen.getAllByRole('listitem')).toHaveLength(3)
    expect(screen.getByText('Connect Telegram')).toBeInTheDocument()
    expect(screen.getByText('Add API key')).toBeInTheDocument()
    expect(screen.getByText('Provisioning')).toBeInTheDocument()
  })

  it('shows a check for completed steps and an ordinal for the rest', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="provisioning" />)
    expect(pills(container)).toEqual(['✓', '✓', '3'])
  })

  it('marks every step before the current one complete, and the rest upcoming', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="apikey" />)
    const states = Array.from(container.querySelectorAll('.mika-stepper__step')).map((el) =>
      el.className.replace('mika-stepper__step ', ''),
    )
    expect(states).toEqual([
      'mika-stepper__step--complete',
      'mika-stepper__step--current',
      'mika-stepper__step--upcoming',
    ])
  })

  it('completes nothing when the first step is current', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="telegram" />)
    expect(pills(container)).toEqual(['1', '2', '3'])
  })

  /**
   * The edge the measured caller actually reaches. `statusToStep` returns
   * `success` while the caller filters `success` out of the displayed list, so
   * `current` names no visible step — and at that moment every visible step
   * genuinely is behind the user.
   *
   * Stated cost, since the two are indistinguishable from inside: a typo in
   * `current` renders the same way.
   */
  it('treats a current step outside the list as past the end', () => {
    const { container } = render(<Stepper steps={MANAGED_TIER} current="success" />)
    expect(pills(container)).toEqual(['✓', '✓'])
    expect(container.querySelector('.mika-stepper__step--current')).toBeNull()
  })

  /**
   * `current` is an id and not an index because the list is tier-dependent:
   * index 1 is `apikey` on one tier and `provisioning` on another.
   */
  it('resolves the same id to different positions across tiers', () => {
    const byoKey = render(<Stepper steps={BYO_KEY_TIER} current="provisioning" />)
    expect(pills(byoKey.container)).toEqual(['✓', '✓', '3'])
    byoKey.unmount()

    const managed = render(<Stepper steps={MANAGED_TIER} current="provisioning" />)
    expect(pills(managed.container)).toEqual(['✓', '2'])
  })

  it('renders nothing but an empty list for no steps', () => {
    const { container } = render(<Stepper steps={[]} current="telegram" />)
    expect(screen.queryAllByRole('listitem')).toHaveLength(0)
    expect(connectors(container)).toHaveLength(0)
  })
})

describe('Stepper — connectors', () => {
  it('draws one connector between each pair of steps, and none before the first', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="telegram" />)
    expect(connectors(container)).toHaveLength(2)
    expect(
      container.querySelector('.mika-stepper__step')?.querySelector('.mika-stepper__connector'),
    ).toBeNull()
  })

  it('fills the trail up to the current step and no further', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="apikey" />)
    const filled = connectors(container).map((el) =>
      el.className.includes('mika-stepper__connector--complete'),
    )
    // telegram → apikey is behind the user; apikey → provisioning is not.
    expect(filled).toEqual([true, false])
  })

  it('fills every connector once progress is past the last visible step', () => {
    const { container } = render(<Stepper steps={MANAGED_TIER} current="success" />)
    expect(
      connectors(container).every((el) =>
        el.className.includes('mika-stepper__connector--complete'),
      ),
    ).toBe(true)
  })
})

describe('Stepper — accessibility', () => {
  it('is a named list, not a navigation landmark', () => {
    const { container } = render(<Stepper steps={BYO_KEY_TIER} current="apikey" />)
    // The steps are driven by server state and none is clickable, so a <nav>
    // landmark here would announce navigation that does not exist.
    expect(screen.getByRole('list', { name: 'Progress' }).tagName).toBe('OL')
    expect(container.querySelector('nav')).toBeNull()
  })

  it('accepts a caller-supplied list name', () => {
    render(<Stepper steps={MANAGED_TIER} current="telegram" ariaLabel="Onboarding progress" />)
    expect(screen.getByRole('list', { name: 'Onboarding progress' })).toBeInTheDocument()
  })

  it('marks exactly the current step with aria-current', () => {
    render(<Stepper steps={BYO_KEY_TIER} current="apikey" />)
    const marked = screen
      .getAllByRole('listitem')
      .filter((el) => el.getAttribute('aria-current') === 'step')
    expect(marked).toHaveLength(1)
    expect(marked[0]).toHaveTextContent('Add API key')
  })

  /**
   * The check glyph and the ordinal are the only carriers of "done" and "not
   * started" on screen, so each has a text equivalent. The current step has
   * none on purpose: `aria-current="step"` already announces it, and a second
   * phrase makes every screen reader say it twice.
   */
  it('gives completed and upcoming steps a text equivalent, and the current one none', () => {
    render(<Stepper steps={BYO_KEY_TIER} current="apikey" />)
    const [done, currentStep, upcoming] = screen.getAllByRole('listitem')
    expect(done).toHaveTextContent('Completed')
    expect(upcoming).toHaveTextContent('Not started')
    expect(currentStep).not.toHaveTextContent('Completed')
    expect(currentStep).not.toHaveTextContent('Not started')
  })

  it('hides the decorative pill from assistive technology', () => {
    const { container } = render(<Stepper steps={MANAGED_TIER} current="provisioning" />)
    container.querySelectorAll('.mika-stepper__pill').forEach((el) => {
      expect(el).toHaveAttribute('aria-hidden', 'true')
    })
  })

  it('merges a caller className onto the list', () => {
    render(<Stepper steps={MANAGED_TIER} current="telegram" className="mb-8" />)
    expect(screen.getByRole('list').className).toContain('mb-8')
  })

  it.each([
    ['first step current', BYO_KEY_TIER, 'telegram'],
    ['middle step current', BYO_KEY_TIER, 'apikey'],
    ['past the last visible step', MANAGED_TIER, 'success'],
  ])('has no axe violations (%s)', async (_name, steps, current) => {
    const { container } = render(<Stepper steps={steps} current={current} />)
    expect(await axe(container)).toHaveNoViolations()
  })
})
