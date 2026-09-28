type StepperStepState = 'complete' | 'current' | 'upcoming'

interface StepperStep {
  /** Stable across tiers — this is what `current` names. */
  id: string
  label: string
}

interface StepperProps {
  /**
   * The steps to display, already filtered by the caller.
   *
   * The measured caller derives them from the tenant's tier
   * (`['telegram','provisioning','success']` for `managed`/`family`, with an
   * extra `apikey` otherwise) and drops the terminal `success` step from the
   * list it shows. That filtering stays in the caller: which steps exist is
   * product knowledge, and a primitive that knew the word `success` would be
   * one tenant tier away from being wrong.
   */
  steps: StepperStep[]
  /**
   * The **id** of the active step, never an index.
   *
   * The list is tier-dependent, so index 1 is `apikey` on one tier and
   * `provisioning` on another: an index would silently point at a different
   * step for a different tenant. An id survives the tier switch.
   *
   * It may legitimately name a step that is **not** in `steps` — the caller
   * filters the terminal step out of the display while `statusToStep` still
   * returns it — and that case is handled below.
   */
  current: string
  /** Names the list for assistive technology. Defaults to `Progress`. */
  ariaLabel?: string
  className?: string
}

/**
 * Announced only for the two states `aria-current` cannot carry. The current
 * step is already announced as such by `aria-current="step"`, and adding
 * "Current step" beside it makes every screen reader say it twice.
 */
const HIDDEN_STATE_TEXT: Record<StepperStepState, string> = {
  complete: 'Completed',
  current: '',
  upcoming: 'Not started',
}

/**
 * The onboarding progress indicator, implementing the shape of the one measured
 * caller (`mika-cloud` `web/src/pages/Onboarding.tsx`, function `StepProgress`,
 * read at `origin/main` 5cb2544).
 *
 * **It is a list, not navigation.** The measured caller drives the active step
 * from server state (`statusToStep`) and no pill is clickable, so this renders
 * an `<ol>` rather than the `<nav aria-label="Progress">` a stepper usually
 * gets: a `<nav>` landmark containing nothing navigable is a lie told to
 * assistive technology. That also answers mika#2562 §5's question — *"les étapes
 * passées sont-elles cliquables ?"* — by measurement: no, and there is
 * therefore no `onStepClick` axis to discriminate on. Nor a per-step error
 * state; the caller has none.
 *
 * Presentation lives in `theme.css` (`.mika-stepper*`), like `<Button>`'s and
 * for the same measured reason: Tailwind does not scan `packages/ui`.
 */
export default function Stepper({
  steps,
  current,
  ariaLabel = 'Progress',
  className = '',
}: StepperProps) {
  const currentIndex = steps.findIndex((step) => step.id === current)

  /**
   * `current` naming no displayed step means progress has moved past all of
   * them — the measured caller's terminal `success`, which it filters out of
   * the display while still reporting it as current. Treating that as "past the
   * end" is what makes the last visible step read as done on the success
   * screen, instead of stalling as current forever.
   *
   * The cost, stated rather than discovered: a typo in `current` renders every
   * step complete instead of raising. The two are indistinguishable from here,
   * and of the two readings only this one is correct for a caller that exists.
   */
  const activeIndex = currentIndex === -1 ? steps.length : currentIndex

  return (
    <ol className={`mika-stepper ${className}`.trim()} aria-label={ariaLabel}>
      {steps.map((step, index) => {
        const state: StepperStepState =
          index < activeIndex ? 'complete' : index === activeIndex ? 'current' : 'upcoming'
        const hiddenText = HIDDEN_STATE_TEXT[state]

        // The connector belongs to the step it leads *into* — so the first step
        // has none — and it is filled once the step behind it is done, which is
        // what stops the filled trail exactly where the user is.
        const connectorClass =
          index === 0
            ? null
            : `mika-stepper__connector${
                index - 1 < activeIndex ? ' mika-stepper__connector--complete' : ''
              }`

        return (
          <li
            key={step.id}
            className={`mika-stepper__step mika-stepper__step--${state}`}
            aria-current={state === 'current' ? 'step' : undefined}
          >
            {connectorClass ? <span className={connectorClass} aria-hidden="true" /> : null}
            <span className="mika-stepper__marker">
              {/* Wholly decorative: the check glyph and the ordinal both restate
                  what the label plus the list's own numbering already convey. */}
              <span
                className={`mika-stepper__pill mika-stepper__pill--${state}`}
                aria-hidden="true"
              >
                {state === 'complete' ? '✓' : index + 1}
              </span>
              <span className="mika-stepper__label">{step.label}</span>
              {hiddenText ? <span className="mika-sr-only">{hiddenText}</span> : null}
            </span>
          </li>
        )
      })}
    </ol>
  )
}

export type { StepperStep, StepperStepState, StepperProps }
