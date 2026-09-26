import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import { CopyButton } from '@samidarko/ui'

describe('CopyButton', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('renders Copy icon visible by default', () => {
    render(<CopyButton text="abc" />)
    const copyIcon = screen.getByTestId('copy-icon')
    const checkIcon = screen.getByTestId('check-icon')
    expect(copyIcon.classList.toString()).toContain('opacity-100')
    expect(checkIcon.classList.toString()).toContain('opacity-0')
  })

  it('crossfades to Check icon after click', async () => {
    render(<CopyButton text="abc" />)
    const button = screen.getByTestId('copy-button')

    await act(async () => {
      fireEvent.click(button)
    })

    const copyIcon = screen.getByTestId('copy-icon')
    const checkIcon = screen.getByTestId('check-icon')
    expect(checkIcon.classList.toString()).toContain('opacity-100')
    expect(copyIcon.classList.toString()).toContain('opacity-0')
  })

  it('reverts to Copy icon after timeout', async () => {
    render(<CopyButton text="abc" />)
    const button = screen.getByTestId('copy-button')

    await act(async () => {
      fireEvent.click(button)
    })

    act(() => {
      vi.advanceTimersByTime(2100)
    })

    const copyIcon = screen.getByTestId('copy-icon')
    const checkIcon = screen.getByTestId('check-icon')
    expect(copyIcon.classList.toString()).toContain('opacity-100')
    expect(checkIcon.classList.toString()).toContain('opacity-0')
  })

  it('calls clipboard.writeText with the text prop', async () => {
    render(<CopyButton text="hello world" />)
    const button = screen.getByTestId('copy-button')

    await act(async () => {
      fireEvent.click(button)
    })

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('hello world')
  })

  it('stops event propagation', async () => {
    const parentHandler = vi.fn()
    render(
      <div onClick={parentHandler}>
        <CopyButton text="abc" />
      </div>,
    )
    const button = screen.getByTestId('copy-button')

    await act(async () => {
      fireEvent.click(button)
    })

    expect(parentHandler).not.toHaveBeenCalled()
  })
})
