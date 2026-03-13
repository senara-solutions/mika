---
title: "feat(tui): support backslash and Shift+Enter for multi-line input"
type: feat
status: completed
date: 2026-03-13
---

# feat(tui): support backslash and Shift+Enter for multi-line input

## Overview

The TUI message composer should support typing backslash (`\`) and using Shift+Enter to insert newlines for multi-line messages. Investigation reveals the code already handles both correctly — the real issue is **terminal compatibility** (Shift+Enter is indistinguishable from Enter on many terminals) and **discoverability** (no visual hint).

## Problem Statement

1. **Shift+Enter**: Many terminal emulators (older xterm, some gnome-terminal versions, Windows Terminal pre-2024) send identical escape sequences for Enter and Shift+Enter. crossterm cannot distinguish them, so the Shift guard at `input.rs:606` never fires — the message sends immediately.
2. **Backslash**: Code analysis confirms backslash (`\`) already works — it passes through to `tui-textarea` with no interception. If users report it being dropped, it's a terminal-specific issue, not a code bug.
3. **Discoverability**: No visual hint that multi-line input is possible.

## Proposed Solution

### 1. Add Alt+Enter as fallback newline keybinding

Alt modifier detection is universally supported across terminals, unlike Shift+Enter.

**File: `crates/mika-cli/src/tui/input.rs`**

Change the Enter guard at line 606 from:
```rust
if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
```
to:
```rust
if key.code == KeyCode::Enter
    && !key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
{
```

Same change in the autocomplete handler at line 388:
```rust
KeyCode::Enter if !key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
```

This preserves existing Shift+Enter support while adding Alt+Enter as a universally-compatible alternative.

### 2. Dismiss autocomplete on Shift/Alt+Enter

When Shift+Enter or Alt+Enter is pressed during autocomplete mode, dismiss the popup before inserting the newline (since multi-line input is incompatible with slash command completion).

**File: `crates/mika-cli/src/tui/input.rs`**

Add a case before the `_` catch-all in `handle_key_autocomplete`:
```rust
// Shift+Enter or Alt+Enter: dismiss autocomplete, insert newline
KeyCode::Enter if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
    app.autocomplete.dismiss();
    app.textarea.input(key);
}
```

### 3. Add visual hint in textarea placeholder

**File: `crates/mika-cli/src/tui/app.rs`** (in `reset_textarea()`)

Update the placeholder text to include the multi-line hint, e.g.:
```
"Type a message... (Shift+Enter or Alt+Enter for new line)"
```

Or add a footer hint in the input area border title showing the keybinding.

### 4. Verify backslash works (no code change expected)

Add a test confirming backslash key events pass through to the textarea unchanged. This serves as a regression guard.

## Acceptance Criteria

- [x] Alt+Enter inserts a newline in the message composer (normal mode)
- [x] Shift+Enter continues to insert a newline (on supporting terminals)
- [x] Neither Alt+Enter nor Shift+Enter sends the message
- [x] Alt+Enter in autocomplete mode dismisses the popup and inserts a newline
- [x] Backslash (`\`) is typeable and appears in the input field
- [x] Multi-line messages are sent correctly with preserved newlines
- [x] History recall preserves multi-line message structure
- [x] Visual hint indicates multi-line keybinding

## Context

### Key files
- `crates/mika-cli/src/tui/input.rs` — keyboard event handlers (`handle_key_normal` line 606, `handle_key_autocomplete` line 388)
- `crates/mika-cli/src/tui/app.rs` — `reset_textarea()`, `send_message()`, `input_text()`
- `crates/mika-cli/src/tui/event.rs` — crossterm event reader (no changes needed)

### Related
- Issue: #132
- Dashboard Shift+Enter solution: `docs/solutions/ui-bugs/dashboard-investigation-panel-shift-enter-newline.md`

## Sources

- crossterm key event limitation: Shift+Enter reports as plain Enter on many terminals
- tui-textarea v0.7: `Input { key: Key::Enter, .. }` pattern matches all Enter variants for newline insertion
