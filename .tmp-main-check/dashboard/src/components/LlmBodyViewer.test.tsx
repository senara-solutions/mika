import { describe, it, expect } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import LlmBodyViewer from './LlmBodyViewer.tsx'

function renderViewer(props: { responseText: string | null; reasoning: string | null; llmCallId: string }) {
  return render(
    <MemoryRouter>
      <LlmBodyViewer {...props} />
    </MemoryRouter>,
  )
}

describe('LlmBodyViewer', () => {
  it('renders response text in a pre-formatted block', () => {
    renderViewer({ responseText: 'Hello world', reasoning: null, llmCallId: 'abc' })
    expect(screen.getByText('Response')).toBeTruthy()
    expect(screen.getByText('Hello world')).toBeTruthy()
  })

  it('renders reasoning section collapsed by default', () => {
    renderViewer({ responseText: null, reasoning: 'Thinking...', llmCallId: 'abc' })
    expect(screen.getByText('Reasoning')).toBeTruthy()
    // Reasoning text should not be visible until expanded
    expect(screen.queryByText('Thinking...')).toBeNull()
  })

  it('expands reasoning on click', () => {
    renderViewer({ responseText: null, reasoning: 'Thinking...', llmCallId: 'abc' })
    fireEvent.click(screen.getByText('Reasoning'))
    expect(screen.getByText('Thinking...')).toBeTruthy()
  })

  it('pretty-prints valid JSON response', () => {
    const json = '{"key":"value","nested":{"a":1}}'
    renderViewer({ responseText: json, reasoning: null, llmCallId: 'abc' })
    // Should be pretty-printed with indentation
    const pre = screen.getByText(/key/)
    expect(pre.textContent).toContain('"key": "value"')
  })

  it('renders invalid JSON as plain text', () => {
    const text = 'This is not JSON { broken'
    renderViewer({ responseText: text, reasoning: null, llmCallId: 'abc' })
    expect(screen.getByText(text)).toBeTruthy()
  })

  it('shows truncation indicator for large bodies', () => {
    const longText = 'x'.repeat(15_000)
    renderViewer({ responseText: longText, reasoning: null, llmCallId: 'abc' })
    expect(screen.getByText(/Showing 10,000 of 15,000 characters/)).toBeTruthy()
    expect(screen.getByText('Show all')).toBeTruthy()
  })

  it('expands full body on "Show all" click', () => {
    const longText = 'x'.repeat(15_000)
    renderViewer({ responseText: longText, reasoning: null, llmCallId: 'abc' })
    fireEvent.click(screen.getByText('Show all'))
    expect(screen.getByText(/15,000 characters/)).toBeTruthy()
    expect(screen.getByText('Truncate')).toBeTruthy()
  })

  it('shows empty state when both fields are null', () => {
    renderViewer({ responseText: null, reasoning: null, llmCallId: 'abc' })
    expect(screen.getByText('No response content available')).toBeTruthy()
  })

  it('renders both sections when both are present', () => {
    renderViewer({ responseText: 'Response text', reasoning: 'Reasoning text', llmCallId: 'abc' })
    expect(screen.getByText('Response')).toBeTruthy()
    expect(screen.getByText('Reasoning')).toBeTruthy()
    // Response should be visible, reasoning collapsed
    expect(screen.getByText('Response text')).toBeTruthy()
    expect(screen.queryByText('Reasoning text')).toBeNull()
  })

  it('renders "View Full Details" link with correct href', () => {
    renderViewer({ responseText: 'text', reasoning: null, llmCallId: 'call-123' })
    const link = screen.getByText(/View Full Details/)
    expect(link.closest('a')?.getAttribute('href')).toBe('/llm-calls/call-123')
  })
})
