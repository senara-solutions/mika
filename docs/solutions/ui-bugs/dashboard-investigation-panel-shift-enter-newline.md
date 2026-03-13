---
title: "Fix Shift+Enter newline support in investigation panel input"
date: 2026-03-13
module: dashboard
problem_type: ui-bugs
severity: medium
tags: [textarea, keyboard-handling, shift-enter, auto-grow, investigation-panel]
related_issues: ["#129"]
---

## Problem

The investigation panel's input field used `<input type="text">`, which is single-line only. Pressing Shift+Enter submitted the form instead of inserting a newline. Users could not write multi-line investigation queries.

## Root Cause

`<input type="text">` does not support newlines. No `onKeyDown` handler existed to distinguish Enter (submit) from Shift+Enter (newline). All submission went through `<form onSubmit>`, which fired on any Enter keypress.

## Solution

Replaced `<input>` with `<textarea>` in `dashboard/src/components/InvestigationPanel.tsx`:

1. **Auto-grow textarea:** `onChange` resets `style.height` to `'auto'` then sets to `scrollHeight + 'px'`, capped by `max-h-[120px]`. Starts at `rows={1}`.
2. **Keyboard handler:** `onKeyDown` intercepts Enter. Plain Enter submits via `sendQuestion()`. Shift+Enter falls through to default behavior (newline insertion).
3. **IME guard:** `!e.nativeEvent.isComposing` prevents premature submission during CJK character composition.
4. **Height reset on submit:** Collapses textarea back to one row after sending.
5. **Layout:** Form container changed from `items-center` to `items-end` so the send button stays at the bottom as textarea grows.

### Key Code

```tsx
<textarea
  ref={inputRef}
  rows={1}
  value={input}
  onChange={(e) => {
    setInput(e.target.value)
    e.target.style.height = 'auto'
    e.target.style.height = e.target.scrollHeight + 'px'
  }}
  onKeyDown={(e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      if (input.trim()) sendQuestion(input)
    }
  }}
  placeholder="Ask a question..."
  disabled={streaming}
  className="... resize-none overflow-y-auto max-h-[120px]"
/>
```

## Prevention

For future multi-line text inputs in the dashboard:

1. Use `<textarea>` with `rows={1}` instead of `<input type="text">` when newlines are expected.
2. Auto-grow via `scrollHeight` in `onChange`, with `max-h-[...]` and `resize-none overflow-y-auto`.
3. Split Enter (submit) from Shift+Enter (newline) via `onKeyDown`.
4. Always guard with `!e.nativeEvent.isComposing` for CJK IME support.
5. Reset `style.height` to `'auto'` after clearing the input on submit.
6. Use `items-end` on the parent flex container so adjacent buttons anchor to the bottom.

## Related

- [Investigation panel SSE architecture](../architecture/investigation-panel-sse-agent-loop.md)
- [TUI textarea selection and mouse support](tui-textarea-selection-rendering-and-mouse-support.md)
- [TUI persistent input history and paste cursor](tui-persistent-history-and-paste-cursor.md)
