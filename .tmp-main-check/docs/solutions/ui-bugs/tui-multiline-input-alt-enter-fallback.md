---
title: "TUI multi-line input: Alt+Enter fallback for terminal Shift+Enter limitations"
date: 2026-03-13
module: mika-cli
problem_type:
  - ui-bug
  - feature-enhancement
severity: medium
tags:
  - crossterm
  - ratatui
  - tui-input
  - terminal-compatibility
  - keyboard-handling
  - placeholder-duplication
related_issues:
  - "#132"
---

# TUI multi-line input: Alt+Enter fallback for terminal Shift+Enter limitations

## Problem

The TUI message composer supported Shift+Enter for multi-line input, but it didn't work on many terminal emulators. Users reported being unable to enter multi-line messages or type backslashes.

**Symptoms:**
- Pressing Shift+Enter sends the message instead of inserting a newline
- No visual indication that multi-line input is possible

**Affected terminals:** Older xterm, some GNOME Terminal versions, Windows Terminal pre-2024, and other terminals that send identical escape sequences for Enter and Shift+Enter.

## Root Cause

crossterm (the terminal event library) reports identical `KeyEvent` values for Enter and Shift+Enter on many terminal emulators. The existing guard at `input.rs:606`:

```rust
if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
```

was logically correct but ineffective on these terminals — `KeyModifiers::SHIFT` was never set, so the guard always passed and the message was sent.

Backslash (`\`) was a false report — it passes through to `tui-textarea` with no interception.

**Secondary issues found during review:**
1. The Esc handler in `input.rs` manually constructed a `TextArea` with a stale placeholder string instead of calling `reset_textarea()`
2. The custom placeholder renderer in `ui.rs:896` had a hardcoded `"Type a message..."` string that overrode `tui-textarea`'s built-in placeholder — making all `set_placeholder_text()` calls in `app.rs` effectively dead code
3. Placeholder strings were duplicated 7 times across `app.rs`, `input.rs`, and `ui.rs`
4. `history_previous()` and `history_next()` hardcoded the normal-mode placeholder, ignoring team mode

## Solution

### 1. Add Alt+Enter as universal fallback

Alt modifier detection works across all terminals, unlike Shift on Enter.

**`crates/mika-cli/src/tui/input.rs` — Enter guard (normal mode and autocomplete mode):**

```rust
// Before
if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {

// After
if key.code == KeyCode::Enter
    && !key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
{
```

Uses `intersects()` (any bit set) instead of `contains()` (all bits set) — correct for "either Shift or Alt" semantics.

### 2. Dismiss autocomplete on Shift/Alt+Enter

Multi-line input is incompatible with slash command completion. Added explicit match arm:

```rust
KeyCode::Enter
    if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
{
    app.autocomplete.dismiss();
    app.textarea.input(key);
}
```

### 3. Extract placeholder constants

Eliminated 7x duplication by introducing two constants in `app.rs`:

```rust
pub const PLACEHOLDER_MESSAGE: &str =
    "Type a message... (Alt+Enter for new line)";
pub const PLACEHOLDER_TEAM_GOAL: &str =
    "Type a goal for the team... (Alt+Enter for new line)";
```

Used in: `App::new()`, `App::new_team()`, `reset_textarea()`, `history_previous()`, `history_next()`, and `ui.rs` custom renderer.

### 4. Fix Esc handler

Replaced inline TextArea reconstruction with `reset_textarea()` call:

```rust
// Before (stale placeholder, not team-mode-aware)
app.textarea = tui_textarea::TextArea::default();
app.textarea.set_cursor_line_style(Style::default());
app.textarea.set_placeholder_text("Type a message...");

// After
app.reset_textarea();
```

### 5. Fix custom placeholder renderer in ui.rs

Updated the renderer that users actually see to use constants and respect team mode:

```rust
let hint = if app.is_team_mode() {
    super::app::PLACEHOLDER_TEAM_GOAL
} else {
    super::app::PLACEHOLDER_MESSAGE
};
let placeholder = Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)));
```

### 6. Make history navigation team-mode-aware

Both `history_previous()` and `history_next()` now select the correct placeholder based on `is_team_mode()`.

## Decision: Alt+Enter as primary multi-line input method

**Chose Alt+Enter as the sole advertised method.** Shift+Enter remains in the code as best-effort but is no longer shown in placeholder text or documented as primary.

**Alternative considered: Kitty keyboard protocol.** crossterm 0.28 supports `PushKeyboardEnhancementFlags` which would enable proper Shift+Enter detection on modern terminals (kitty, foot, WezTerm, recent alacritty). Rejected because:

1. It changes how ALL key events are reported — high risk of breaking existing key handling across the TUI
2. Still wouldn't work on non-supporting terminals (basic xterm, older GNOME Terminal)
3. Would require extensive cross-terminal testing
4. Not worth the complexity given Alt+Enter already works universally

**Backslash line continuation** was a false report (see Root Cause above). Backslash passes through as a literal character — no change needed.

Placeholder text updated from `"(Shift+Enter or Alt+Enter for new line)"` to `"(Alt+Enter for new line)"`. Documentation updated to list Alt+Enter as primary, Shift+Enter as secondary with terminal caveat.

## Prevention Strategies

1. **Extract UI string constants immediately** when a string appears in more than one location. Duplication across constructors, handlers, and renderers will drift.

2. **Always check the actual rendering path.** Custom renderers (like `ui.rs` draw functions) can silently override widget library behavior (like `tui-textarea`'s built-in placeholder). When updating a widget property, trace where the rendered output actually comes from.

3. **Provide fallback keybindings for critical features.** Terminal keybinding compatibility varies widely — Alt modifier is universally detected, unlike Shift on Enter. Default to universally-compatible keybindings.

4. **Use existing helper methods for state resets.** The Esc handler manually reconstructed a TextArea instead of calling `reset_textarea()`. Inline reconstruction diverges from the canonical reset path over time.

5. **Make mode-dependent code explicit.** When placeholder text or behavior depends on mode (chat vs team), pass the mode context through rather than hardcoding one variant.

## Related Documentation

- [Dashboard Shift+Enter solution](dashboard-investigation-panel-shift-enter-newline.md) — parallel React implementation with IME guard
- [TUI persistent history and paste cursor](tui-persistent-history-and-paste-cursor.md) — textarea `insert_str()` best practice
- [TUI shell-like autocompletion](tui-shell-like-autocompletion.md) — `CompletionMode` state machine context
- [Team TUI mode integration](../integration-issues/team-tui-mode-cli-integration.md) — team mode placeholder patterns

## Files Changed

- `crates/mika-cli/src/tui/input.rs` — Enter guard, autocomplete dismiss, Esc handler fix
- `crates/mika-cli/src/tui/app.rs` — Constants, `reset_textarea()` visibility, history navigation
- `crates/mika-cli/src/tui/ui.rs` — Custom placeholder renderer
- `CLAUDE.md` — TUI keybinding documentation
- `docs/slash-commands.md` — Keybinding tables
