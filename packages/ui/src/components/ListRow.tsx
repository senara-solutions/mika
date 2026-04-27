import { useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react'
import { ChevronRight, ChevronDown } from 'lucide-react'

type ListRowVariant = 'static' | 'navigable' | 'expandable'

interface ListRowBaseProps {
  children: ReactNode
  className?: string
}

interface StaticProps extends ListRowBaseProps {
  variant: 'static'
}

interface NavigableProps extends ListRowBaseProps {
  variant: 'navigable'
  onClick: () => void
  ariaLabel?: string
}

interface ExpandableProps extends ListRowBaseProps {
  variant: 'expandable'
  isExpanded: boolean
  onToggle: () => void
  ariaLabel?: string
}

type ListRowProps = StaticProps | NavigableProps | ExpandableProps

function isTargetRow(e: MouseEvent | KeyboardEvent, rowRef: React.RefObject<HTMLTableRowElement | null>): boolean {
  const target = e.target as HTMLElement
  // Don't trigger row action if the click/key originated from a nested interactive element
  if (target.closest('a, button, [role="button"], [role="link"]') && target.closest('a, button, [role="button"], [role="link"]') !== rowRef.current) {
    return false
  }
  return true
}

export default function ListRow(props: ListRowProps) {
  const rowRef = useRef<HTMLTableRowElement>(null)
  const { variant, children, className = '' } = props

  const baseClass = 'hover:bg-white/[0.02] transition-colors'

  if (variant === 'static') {
    return (
      <tr data-list-row className={`${baseClass} ${className}`}>
        {children}
      </tr>
    )
  }

  if (variant === 'navigable') {
    const { onClick, ariaLabel } = props

    const handleClick = (e: MouseEvent<HTMLTableRowElement>) => {
      if (!isTargetRow(e, rowRef)) return
      onClick()
    }

    const handleKeyDown = (e: KeyboardEvent<HTMLTableRowElement>) => {
      if (!isTargetRow(e, rowRef)) return
      if (e.key === 'Enter') {
        e.preventDefault()
        onClick()
      }
    }

    return (
      <tr
        ref={rowRef}
        data-list-row
        className={`${baseClass} cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent/40 focus-visible:outline-offset-[-2px] ${className}`}
        onClick={handleClick}
        onKeyDown={handleKeyDown}
        tabIndex={0}
        role="link"
        aria-label={ariaLabel}
      >
        <td className="px-2 py-3 text-accent/40">
          &rarr;
        </td>
        {children}
      </tr>
    )
  }

  // expandable
  const { isExpanded, onToggle, ariaLabel } = props

  const handleClick = (e: MouseEvent<HTMLTableRowElement>) => {
    if (!isTargetRow(e, rowRef)) return
    onToggle()
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLTableRowElement>) => {
    if (!isTargetRow(e, rowRef)) return
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onToggle()
    } else if (e.key === 'Escape' && isExpanded) {
      e.preventDefault()
      onToggle()
    }
  }

  return (
    <tr
      ref={rowRef}
      data-list-row
      className={`${baseClass} cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent/40 focus-visible:outline-offset-[-2px] ${className}`}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      tabIndex={0}
      role="button"
      aria-expanded={isExpanded}
      aria-label={ariaLabel}
    >
      <td className="px-2 py-3 text-muted/30">
        {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
      </td>
      {children}
    </tr>
  )
}

export type { ListRowVariant, ListRowProps }
