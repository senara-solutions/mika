import { useId, useState } from 'react'
import Button from './Button.tsx'

interface SecretFieldProps {
  /**
   * Rendered as a `<label>` above the field and wired to it by id.
   *
   * Also composed into the reveal control's accessible name ("Show Anthropic
   * API key"), which is what keeps two secret fields on one page tellable
   * apart — the measured caller's bare `Show key` would not.
   */
  label: string
  value: string
  onChange: (value: string) => void
  placeholder?: string
  /**
   * Defaults to `off`, the measured caller's value.
   *
   * This is the resolution of the question mika#2562 §5 raised — *"`autocomplete`
   * vaut-il `current-password` ou `new-password`, et si les deux, l'axe est-il
   * une prop ou deux composants ?"* — and the callsite answers it by refusing
   * the premise: the console's only masked field is an **API key**, and its
   * authentication is entirely Google OAuth, so no password exists anywhere in
   * it. There is no login-vs-signup axis to discriminate on. It stays one flat
   * pass-through prop rather than becoming a union, because a union here would
   * be decorative — which is the trap D1 of LC.2 named.
   */
  autoComplete?: string
  /** Marks the field invalid for assistive technology. */
  invalid?: boolean
  /**
   * `id` of the caller's error or help text.
   *
   * The measured caller renders its field error **below** the input, itself —
   * so this primitive deliberately does not own error rendering. That makes
   * these last two props load-bearing rather than decorative: without them the
   * extraction would leave the error unreachable to a screen reader, which
   * `packages/ui/CLAUDE.md` § Accessibility Standards does not allow.
   */
  describedBy?: string
  className?: string
}

/**
 * The library's masked text field, with a reveal toggle.
 *
 * **Named for what was measured, not for what the ticket guessed.** mika#2562
 * offers `PasswordInput` / `SecretField` as alternatives; the one readable
 * caller (`mika-cloud` `web/src/pages/onboarding/ApiKeysStep.tsx`, read at
 * `origin/main` 5cb2544) masks an **Anthropic API key**, and the console holds
 * no password field at all. `PasswordInput` would have named a population of
 * zero.
 *
 * Presentation lives in `theme.css` (`.mika-field*`), like `<Button>`'s and for
 * the same measured reason: Tailwind does not scan `packages/ui`.
 *
 * Two halves, two sources. The field chrome is rulebook §5 "Input Fields",
 * which exists and is applied literally. The *secret* grammar — masking, the
 * reveal control, monospace, `spellCheck={false}` — has no rulebook section
 * (mika#2562 §2.1 measured that) and comes from the caller. That gap is an
 * operator follow-up for Vincent under §8, not something to invent here.
 *
 * **No discriminated union**, and that is the measured outcome rather than an
 * omission: every axis a union could have keyed on — login vs signup, controlled
 * vs internal reveal, error inside vs outside — is answered the same way by the
 * single caller, so a union would be one interface under several names.
 */
export default function SecretField({
  label,
  value,
  onChange,
  placeholder,
  autoComplete = 'off',
  invalid,
  describedBy,
  className,
}: SecretFieldProps) {
  // Internal, not a prop. The caller holds `showKey` in local state and nothing
  // outside the field reads it, so lifting it would be an axis no one asked for.
  const [revealed, setRevealed] = useState(false)
  const inputId = useId()

  return (
    <div className={className}>
      <label htmlFor={inputId} className="mika-field-label">
        {label}
      </label>
      <div className="mika-field-wrap">
        <input
          id={inputId}
          type={revealed ? 'text' : 'password'}
          className="mika-field mika-field--secret"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          autoComplete={autoComplete}
          spellCheck={false}
          aria-invalid={invalid}
          aria-describedby={describedBy}
        />
        {/*
          `<Button variant="tertiary">` rather than a bare `<button>`: §5 defines
          tertiary as "text-only using `primary` color, no background, for
          low-priority actions", which is this control exactly. Reuse also buys
          the focus ring and — the one that would have bitten — `type="button"`,
          without which this toggle submits the form it sits in.

          The visible text is the caller's ("Show" / "Hide", not an icon), so the
          accessible name starts with it: WCAG 2.5.3 Label in Name.
        */}
        <Button
          variant="tertiary"
          size="sm"
          className="mika-field-reveal"
          onClick={() => setRevealed((shown) => !shown)}
          ariaLabel={`${revealed ? 'Hide' : 'Show'} ${label}`}
        >
          {revealed ? 'Hide' : 'Show'}
        </Button>
      </div>
    </div>
  )
}

export type { SecretFieldProps }
