import { Loader2 } from 'lucide-react'

type SpinnerSize = 'xs' | 'sm' | 'md'

interface SpinnerProps {
  /**
   * 12 / 16 / 20 px. Calibrated on `ButtonSize`, **not** on a rulebook §6
   * scale: §6 is "Roundness & Spacing" and mentions no loader at all. There is
   * no spinner-size scale in the rulebook to implement (mika#1801 R4), so the
   * scale that exists is the one its only caller needs.
   */
  size?: SpinnerSize
  className?: string
  /** Accessible name announced while the spinner is live. */
  ariaLabel?: string
  /**
   * Suppress the live region and hide the icon from assistive technology.
   *
   * This is what `<Button loading>` passes: the button already carries
   * `aria-busy="true"`, and two nested live regions announce the same fact
   * twice. A spinner rendered *inside* something that already says "busy" is
   * decoration.
   */
  decorative?: boolean
}

const SIZE_PX: Record<SpinnerSize, number> = {
  xs: 12,
  sm: 16,
  md: 20,
}

/**
 * The library's one loader.
 *
 * Extracted by LC.2 (mika#1801) as a **dependency of `<Button loading>`**, not
 * on its own population: the repo's only spinners are three identical
 * `<Loader2 className="animate-spin" />` in a single file
 * (`dashboard/src/components/InvestigationPanel.tsx`), which is one surface and
 * therefore under the rulebook §9 sharing threshold. What justifies it in the
 * shared package is the caller that requires it — the ticket's AC1 asks for
 * `loading` "avec Spinner intégré". The implementation is byte-for-byte the
 * shape already in that file, so extraction changes no pixel there.
 */
export default function Spinner({
  size = 'md',
  className = '',
  ariaLabel = 'Loading',
  decorative = false,
}: SpinnerProps) {
  // `.mika-spin` and not Tailwind's `animate-spin`: Tailwind does not scan
  // `packages/ui`, so `animate-spin` reaches the dashboard only because
  // `InvestigationPanel.tsx` happens to use it, and reaches the landing and
  // mika-cloud not at all. The keyframes are declared in `theme.css` beside the
  // button presentation, with the measurement. The animation is identical to
  // what those three call sites render today, so extracting them changes no pixel.
  const icon = (
    <Loader2 size={SIZE_PX[size]} className={`mika-spin ${className}`.trim()} />
  )

  if (decorative) {
    return <span aria-hidden="true">{icon}</span>
  }

  // `role="status"` already implies a polite live region; `aria-live` is written
  // out because `packages/ui/CLAUDE.md` § Accessibility Standards requires the
  // pair explicitly for async state changes.
  return (
    <span role="status" aria-live="polite" aria-label={ariaLabel}>
      {icon}
    </span>
  )
}

export type { SpinnerSize, SpinnerProps }
