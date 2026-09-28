import { render, screen, fireEvent } from '@testing-library/react'
import { describe, it, expect, vi } from 'vitest'
import { axe } from 'jest-axe'
import SecretField from './SecretField'

const LABEL = 'Anthropic API key'

const renderField = (props: Partial<Parameters<typeof SecretField>[0]> = {}) =>
  render(<SecretField label={LABEL} value="" onChange={() => {}} {...props} />)

describe('SecretField — masking and reveal', () => {
  it('masks the value by default', () => {
    renderField()
    expect(screen.getByLabelText(LABEL)).toHaveAttribute('type', 'password')
  })

  it('Show reveals the value, Hide masks it again', () => {
    renderField()
    fireEvent.click(screen.getByRole('button', { name: `Show ${LABEL}` }))
    expect(screen.getByLabelText(LABEL)).toHaveAttribute('type', 'text')

    fireEvent.click(screen.getByRole('button', { name: `Hide ${LABEL}` }))
    expect(screen.getByLabelText(LABEL)).toHaveAttribute('type', 'password')
  })

  it('the control shows its own state, not the action it would undo', () => {
    renderField()
    expect(screen.getByRole('button')).toHaveTextContent('Show')
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByRole('button')).toHaveTextContent('Hide')
  })

  /**
   * The measured caller's `aria-label` is a bare `Show key`. Composing the
   * field's own label is what keeps two secret fields on one page tellable
   * apart, and WCAG 2.5.3 Label in Name is why the visible word comes first.
   */
  it('the accessible name composes the field label, starting with the visible word', () => {
    renderField()
    const control = screen.getByRole('button')
    expect(control).toHaveAccessibleName(`Show ${LABEL}`)
    expect(control.textContent).toBe('Show')
  })

  /**
   * The one reuse that would have bitten if the control were a bare `<button>`:
   * inside a form, a button with no `type` defaults to `submit`, so revealing a
   * secret would submit the form. `<Button>` sets `type="button"`.
   */
  it('revealing does not submit the surrounding form', () => {
    const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault())
    render(
      <form onSubmit={onSubmit}>
        <SecretField label={LABEL} value="" onChange={() => {}} />
      </form>,
    )
    expect(screen.getByRole('button')).toHaveAttribute('type', 'button')
    fireEvent.click(screen.getByRole('button'))
    expect(onSubmit).not.toHaveBeenCalled()
  })
})

describe('SecretField — field contract', () => {
  it('wires the label to the input', () => {
    renderField()
    const input = screen.getByLabelText(LABEL)
    expect(input.id).toBeTruthy()
    expect(screen.getByText(LABEL)).toHaveAttribute('for', input.id)
  })

  it('gives two fields on one page distinct ids', () => {
    render(
      <>
        <SecretField label="First key" value="" onChange={() => {}} />
        <SecretField label="Second key" value="" onChange={() => {}} />
      </>,
    )
    expect(screen.getByLabelText('First key').id).not.toBe(
      screen.getByLabelText('Second key').id,
    )
  })

  it('reports edits through onChange', () => {
    const onChange = vi.fn()
    renderField({ onChange })
    fireEvent.change(screen.getByLabelText(LABEL), { target: { value: 'sk-ant-123' } })
    expect(onChange).toHaveBeenCalledWith('sk-ant-123')
  })

  it('renders the value it is given', () => {
    renderField({ value: 'sk-ant-123' })
    expect(screen.getByLabelText(LABEL)).toHaveValue('sk-ant-123')
  })

  it('renders a placeholder', () => {
    renderField({ placeholder: 'sk-ant-...' })
    expect(screen.getByLabelText(LABEL)).toHaveAttribute('placeholder', 'sk-ant-...')
  })

  /**
   * Both values are the measured caller's. What these assertions pin is the
   * **attributes**, and only `spellCheck` buys its protection outright: it is
   * what keeps a key out of Chrome's Enhanced Spell Check, which uploads field
   * text for checking.
   *
   * `autoComplete="off"` is weaker than it reads, and saying so here is the
   * point: Chromium and WebKit deliberately ignore it on credential-shaped
   * fields so that password managers keep working, so a browser may still offer
   * to save the value. jsdom has no autofill engine, so no test in this file can
   * tell "the attribute is set" from "the browser honours it" — which is exactly
   * why this comment states the former and not the latter. Whether the default
   * should change is an open review finding, carried on the PR rather than
   * decided here.
   */
  it('defaults autoComplete to off and disables spellcheck', () => {
    renderField()
    const input = screen.getByLabelText(LABEL)
    expect(input).toHaveAttribute('autocomplete', 'off')
    expect(input).toHaveAttribute('spellcheck', 'false')
  })

  /** Revealed is the state where a spellchecker would actually run. */
  it('keeps spellcheck off while revealed', () => {
    renderField()
    fireEvent.click(screen.getByRole('button'))
    const input = screen.getByLabelText(LABEL)
    expect(input).toHaveAttribute('type', 'text')
    expect(input).toHaveAttribute('spellcheck', 'false')
  })

  /**
   * The negative that matters most for a secret primitive, and the one property
   * a reader would otherwise have to establish by tracing `value` by hand: the
   * secret reaches the input's own value and nothing else — no `data-*`, no
   * `title`, no accessible name, no preview node. Pins it against a future prop
   * that reintroduces it.
   */
  it('renders the secret nowhere but the input value', () => {
    const secret = 'sk-ant-do-not-leak-me'
    const { container } = renderField({ value: secret })
    const input = screen.getByLabelText(LABEL)
    expect(input).toHaveValue(secret)

    // Every element except the input itself — the label, the wrapper, the
    // reveal control, and any node a future prop adds. React does serialize the
    // input's own `value` attribute, so the assertion is scoped rather than
    // taken over `container.innerHTML`: excluding the one legitimate carrier is
    // what makes a second carrier visible.
    Array.from(container.querySelectorAll('*'))
      .filter((el) => el !== input)
      .forEach((el) => {
        Array.from(el.attributes).forEach((attr) => {
          expect(attr.value).not.toContain(secret)
        })
      })

    // And nothing renders it as text — a preview node would land here.
    expect(container.textContent).not.toContain(secret)
  })

  it('accepts an explicit autoComplete without becoming a second component', () => {
    renderField({ autoComplete: 'current-password' })
    expect(screen.getByLabelText(LABEL)).toHaveAttribute('autocomplete', 'current-password')
  })

  it('carries the secret presentation classes', () => {
    renderField()
    const cls = screen.getByLabelText(LABEL).className
    expect(cls).toContain('mika-field')
    expect(cls).toContain('mika-field--secret')
  })

  /**
   * The caller renders its own error below the field, so these two props are
   * the only thing that makes that error reachable from the input.
   */
  it('wires an invalid state and a caller-owned description', () => {
    render(
      <>
        <SecretField
          label={LABEL}
          value=""
          onChange={() => {}}
          invalid
          describedBy="key-error"
        />
        <p id="key-error">That key is not valid</p>
      </>,
    )
    const input = screen.getByLabelText(LABEL)
    expect(input).toHaveAttribute('aria-invalid', 'true')
    expect(input).toHaveAccessibleDescription('That key is not valid')
  })

  it('omits both attributes when the caller reports no error', () => {
    renderField()
    const input = screen.getByLabelText(LABEL)
    expect(input).not.toHaveAttribute('aria-invalid')
    expect(input).not.toHaveAttribute('aria-describedby')
  })

  it('merges a caller className onto the field group', () => {
    const { container } = renderField({ className: 'mb-4' })
    expect(container.firstElementChild?.className).toContain('mb-4')
  })
})

describe('SecretField — accessibility', () => {
  it('has no axe violations while masked', async () => {
    const { container } = renderField()
    expect(await axe(container)).toHaveNoViolations()
  })

  it('has no axe violations while revealed', async () => {
    const { container } = renderField()
    fireEvent.click(screen.getByRole('button'))
    expect(await axe(container)).toHaveNoViolations()
  })

  it('has no axe violations when invalid and described', async () => {
    const { container } = render(
      <>
        <SecretField
          label={LABEL}
          value=""
          onChange={() => {}}
          invalid
          describedBy="key-error"
        />
        <p id="key-error">That key is not valid</p>
      </>,
    )
    expect(await axe(container)).toHaveNoViolations()
  })
})
