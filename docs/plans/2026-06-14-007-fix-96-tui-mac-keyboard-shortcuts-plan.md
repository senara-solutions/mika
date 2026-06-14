# Plan — fix(tui): Mac keyboard shortcuts not working (mika#96)

## Goal

Add 8 Mac-standard keyboard shortcuts to the Mika TUI input field so text editing feels native on macOS. All shortcuts route through the existing `handle_key_normal` dispatcher in `crates/mika-cli/src/tui/input.rs` and use primitives already exposed by `tui_textarea` — no new dependencies.

## Files

**Modify**
- `crates/mika-cli/src/tui/input.rs` — add 8 keybinding handlers BEFORE the existing fall-through to `app.textarea.input(key)` at function tail.

**Tests**
- `crates/mika-cli/src/tui/input.rs` — inline `#[cfg(test)] mod tests` — 8 per-keypress unit tests.

## Key matrix

All handlers fire BEFORE the fall-through call `app.textarea.input(key)` and `return` after setting `app.needs_redraw = true`.

| Shortcut | Crossterm pattern | `tui_textarea` action |
|---|---|---|
| Alt+Backspace | `ALT + Backspace` | `textarea.delete_word()` |
| Alt+Left | `ALT + Left` | `textarea.move_cursor(CursorMove::WordBack)` |
| Alt+Right | `ALT + Right` | `textarea.move_cursor(CursorMove::WordForward)` |
| Cmd+Left | `SUPER + Left` | `textarea.move_cursor(CursorMove::Head)` |
| Cmd+Right | `SUPER + Right` | `textarea.move_cursor(CursorMove::End)` |
| Cmd+Backspace | `SUPER + Backspace` | `textarea.delete_line_by_head()` |
| Cmd+A | `SUPER + Char('a')` | `textarea.select_all()` (peer to existing Ctrl+A) |
| Ctrl+W | `CONTROL + Char('w')` | `textarea.delete_word()` (POSIX bonus) |

## Approach

Each handler follows the exact shape of the existing Ctrl+A handler at `input.rs:591`:

```rust
if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Backspace {
    app.textarea.delete_word();
    app.needs_redraw = true;
    return;
}
```

Insert handlers between the existing Ctrl+V paste handler (~line 600) and the Esc handler (~line 639). Order within the new block: Alt-modifier handlers first, then Super-modifier handlers, then the Ctrl+W bonus.

For the Cmd+A peer: extend the existing Ctrl+A condition to also accept SUPER:

```rust
if (key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::SUPER))
    && key.code == KeyCode::Char('a') {
    // ... existing select_all body unchanged
}
```

## Test scenarios

8 inline unit tests in `crates/mika-cli/src/tui/input.rs`'s `#[cfg(test)] mod tests`. Each test:
1. Creates an `App` via the existing test-builder pattern (see `test_send_while_healthy_dispatches_to_worker` for reference).
2. Pre-populates `app.textarea` with a known string.
3. Synthesizes a `KeyEvent::new(code, modifiers)`.
4. Calls `handle_key_normal(&mut app, key)`.
5. Asserts on `app.textarea.lines()` content and/or cursor position.

Naming pattern: `test_<shortcut>_<expected_action>`. Example:

```rust
#[test]
fn test_alt_backspace_deletes_previous_word() {
    let mut app = make_test_app();
    app.textarea.insert_str("hello world");
    let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT);
    handle_key_normal(&mut app, key);
    assert_eq!(app.textarea.lines(), &["hello "]);
}
```

Repeat for: alt_left_jumps_word_back, alt_right_jumps_word_forward, cmd_left_jumps_to_line_start, cmd_right_jumps_to_line_end, cmd_backspace_deletes_to_line_start, cmd_a_selects_all, ctrl_w_deletes_previous_word.

## Verification

Per parent AC:
- `cargo test -p mika-cli` passes (all 8 new tests + existing tests).
- `cargo clippy -p mika-cli --tests --no-deps -- -D warnings` clean.
- `cargo build -p mika-cli` clean.

## Out of scope

- Terminal-emulator config (Terminal.app not forwarding Cmd is a user-side setting; this fix relies on terminals that DO forward Cmd as SUPER, e.g. iTerm2 with that option enabled, kitty, WezTerm).
- Non-input surfaces (chat scrollback, message list).
- Customizable keybindings.
- Vim/Emacs modes.

## Risk

LOW. Additive change: 8 new handlers + 1 peer addition to existing Ctrl+A. Existing handlers (Ctrl+C, Ctrl+A, Ctrl+V, Esc, PageUp/Down, Enter, Tab, Ctrl+Up/Down, Up/Down history) are not modified. Falls back to current `app.textarea.input(key)` behavior on any unmatched modifier+key combo.

## Patterns to follow

- Existing Ctrl+A handler at `crates/mika-cli/src/tui/input.rs:591` — handler shape (modifier check → action → needs_redraw → return).
- Existing test pattern `test_send_while_healthy_dispatches_to_worker` at `crates/mika-cli/src/tui/input.rs:~1261` — test scaffolding for `App` and key events.
- `tui_textarea` docs (already imported as `use tui_textarea::CursorMove;` at `input.rs:3`) — `CursorMove`, `delete_word`, `delete_line_by_head`, `select_all` primitives.

## Sequencing

Single PR. No dependencies on other tickets. No deferred follow-ups.
