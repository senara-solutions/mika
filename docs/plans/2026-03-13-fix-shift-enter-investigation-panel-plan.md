---
title: "fix(dashboard): support newline (Shift+Enter) in investigation panel input"
type: fix
status: completed
date: 2026-03-13
issue: "#129"
---

# fix(dashboard): support newline (Shift+Enter) in investigation panel input

Shift+Enter in the investigation panel input submits the message instead of inserting a newline. Users cannot write multi-line investigation prompts.

## Root Cause

`InvestigationPanel.tsx` uses `<input type="text">` (line 281) which is single-line only. No `onKeyDown` handler exists — all submission goes through `<form onSubmit={handleSubmit}>`. Since `<input>` doesn't support newlines, Shift+Enter triggers form submission like plain Enter.

## Acceptance Criteria

- [x] Shift+Enter inserts a newline in the investigation input
- [x] Enter (without Shift) submits the message
- [x] Textarea auto-grows from 1 row up to ~5 rows, then scrolls internally
- [x] Textarea height resets to 1 row after submission
- [x] IME composition (CJK) does not trigger premature submission (`isComposing` guard)
- [x] Existing message display preserves newlines (already uses `whitespace-pre-wrap`)
- [x] TypeScript compiles without errors (ref type updated)

## Implementation

### `dashboard/src/components/InvestigationPanel.tsx`

1. **Update ref type** (line 33): `useRef<HTMLInputElement>` → `useRef<HTMLTextAreaElement>`

2. **Replace `<input>` with `<textarea>`** (lines 281-289):
   - Add `rows={1}` for single-line start
   - Add `resize-none overflow-y-auto` classes, set `max-h-[120px]`
   - Add `onKeyDown` handler:
     ```tsx
     onKeyDown={(e) => {
       if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
         e.preventDefault();
         if (input.trim()) sendQuestion(input);
       }
     }}
     ```

3. **Auto-grow logic**: In `onChange`, reset `style.height` to `auto` then set to `scrollHeight + 'px'` (capped at 120px via CSS `max-h-[120px]`).

4. **Reset height on submit**: After `setInput('')` in `sendQuestion`, reset textarea height to initial.

5. **Form container alignment** (line 279): Change `items-center` → `items-end` so the send button stays at the bottom when textarea grows.

## Context

- File: `dashboard/src/components/InvestigationPanel.tsx`
- Message display already uses `whitespace-pre-wrap` (line 250) — multiline messages render correctly
- Server-side `question` field is a plain string; newlines in JSON are standard and handled by Claude API
- No other textarea exists in the dashboard — this is the first multi-line input

## Sources

- GitHub issue: #129
- Solution doc: `docs/solutions/architecture/investigation-panel-sse-agent-loop.md`
