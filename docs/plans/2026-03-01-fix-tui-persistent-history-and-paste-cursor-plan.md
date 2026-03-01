---
title: Fix TUI Persistent Input History and Paste Cursor Positioning
type: fix
status: completed
date: 2026-03-01
---

# Fix TUI Persistent Input History and Paste Cursor Positioning

## Overview

Two TUI input handling bugs that degrade the chat experience:

1. **Input history lost on exit** — The `InputHistory` struct in `crates/mika-cli/src/tui/app.rs` is purely in-memory. When the user quits and relaunches the TUI, all Up/Down arrow history is gone.
2. **Paste inserts at wrong position** — `handle_paste()` in `crates/mika-cli/src/tui/input.rs:188` always appends pasted text to the end of the last line, ignoring cursor position. After paste, the cursor jumps to position (0,0).

## Problem Statement

**History loss:** Users expect shell-like behavior where input history persists across sessions. Currently, `InputHistory::new()` creates an empty `Vec<String>` with no disk I/O. The `push()` method only appends to the in-memory vec.

**Paste cursor bug:** The `handle_paste()` function reconstructs the entire `TextArea` from scratch by appending pasted text to the last line, discarding cursor position. `tui-textarea` 0.7 provides `insert_str()` which inserts at the current cursor position — the current code bypasses this.

## Proposed Solution

### Fix 1: Persistent Input History

- Store history as JSON array in `{home_dir}/.input_history` (per-agent)
- Load on `InputHistory` construction via `InputHistory::load(home_dir)`
- Save atomically after each `push()` (write to `.input_history.tmp`, then `rename`)
- Set file permissions to `0600` (matches `mcp.json` precedent for potentially sensitive data)
- On corrupt/missing file: log warning, start with empty history
- On save failure: log warning, continue without crashing
- Reload history on agent switch in `chat.rs`

### Fix 2: Paste Cursor Positioning

- Replace the body of `handle_paste()` with `app.textarea.insert_str(text)`
- Normalize `\r\n` → `\n` and bare `\r` → `\n` before insertion
- Add paste size limit (100KB) with user warning for larger pastes
- Skip empty pastes early

## Technical Considerations

- **Atomic writes** prevent corruption if the process is killed mid-write
- **Per-agent history** (using `app.home_dir`) keeps context-specific messages separate across agents
- **`insert_str`** preserves undo history — Ctrl+Z will undo a paste (currently impossible since the TextArea is reconstructed)
- **Agent switch** code in `chat.rs:325-393` currently does NOT reload `app.history` when `app.home_dir` changes — must be fixed
- **`CursorMove::Jump` uses `u16`** — not needed for paste fix since `insert_str` handles cursor positioning internally
- **Concurrency:** Last-writer-wins for the history file (matches bash/zsh behavior). No file locking needed.

## Acceptance Criteria

### Persistent History
- [x] History entries survive TUI exit and relaunch
- [x] First launch with no history file works (empty history)
- [x] Corrupt/invalid history file does not crash TUI — starts fresh with warning log
- [x] History is capped at 500 entries (existing behavior)
- [x] Slash commands are NOT stored in history (existing behavior preserved)
- [x] Multi-line entries round-trip correctly through JSON serialization
- [x] Agent switch via `/agent` reloads history from the new agent's home dir
- [x] History file has 0600 permissions

### Paste Cursor Fix
- [x] Pasting text inserts at the current cursor position, not at the end
- [x] Cursor is positioned at the end of the pasted text after paste
- [x] Multi-line paste works correctly (splits lines at cursor)
- [x] Empty paste is a no-op
- [x] Undo (Ctrl+Z) reverses a paste operation
- [x] Windows line endings (`\r\n`) are normalized to `\n`
- [x] Pastes larger than 100KB are truncated with a system message warning

## MVP

### `crates/mika-cli/src/tui/app.rs` — InputHistory persistence

```rust
use std::path::{Path, PathBuf};

const HISTORY_FILENAME: &str = ".input_history";

pub struct InputHistory {
    entries: Vec<String>,
    index: Option<usize>,
    saved_draft: Option<String>,
    /// Path to the history file (None = no persistence, e.g. in tests).
    file_path: Option<PathBuf>,
}

impl InputHistory {
    /// Load history from disk, or start empty if file is missing/corrupt.
    pub fn load(home_dir: &Path) -> Self {
        let file_path = home_dir.join(HISTORY_FILENAME);
        let entries = Self::read_file(&file_path).unwrap_or_default();
        Self {
            entries,
            index: None,
            saved_draft: None,
            file_path: Some(file_path),
        }
    }

    /// In-memory only (for tests).
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: None,
            saved_draft: None,
            file_path: None,
        }
    }

    pub fn push(&mut self, entry: String) {
        if !entry.is_empty() {
            self.entries.push(entry);
            if self.entries.len() > HISTORY_MAX_SIZE {
                self.entries.remove(0);
            }
        }
        self.index = None;
        self.saved_draft = None;
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.file_path else { return };
        if let Err(e) = Self::write_file(path, &self.entries) {
            tracing::warn!("failed to save input history: {e}");
        }
    }

    fn read_file(path: &Path) -> Option<Vec<String>> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn write_file(path: &Path, entries: &[String]) -> std::io::Result<()> {
        let json = serde_json::to_string(entries)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        // Set permissions before rename (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
```

### `crates/mika-cli/src/tui/input.rs` — Fixed paste handler

```rust
/// Maximum paste size (100KB) to prevent UI freezing.
const MAX_PASTE_BYTES: usize = 100 * 1024;

pub fn handle_paste(app: &mut App<'_>, text: &str) {
    if text.is_empty() {
        return;
    }

    let text = if text.len() > MAX_PASTE_BYTES {
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!(
                "Paste truncated to {}KB (was {}KB).",
                MAX_PASTE_BYTES / 1024,
                text.len() / 1024
            ),
            rendered: None,
            channel: None,
        });
        &text[..text.floor_char_boundary(MAX_PASTE_BYTES)]
    } else {
        text
    };

    // Normalize line endings: \r\n → \n, bare \r → \n
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    app.textarea.insert_str(&normalized);
    app.needs_redraw = true;
}
```

### `crates/mika-cli/src/tui/app.rs` — App::new() change

```rust
// In App::new(), change:
//   history: InputHistory::new(),
// To:
    history: InputHistory::load(&home_dir),
```

### `crates/mika-cli/src/commands/chat.rs` — Agent switch history reload

```rust
// After line 354 (app.home_dir = new_worker._ctx.home_dir.clone();), add:
app.history = InputHistory::load(&app.home_dir);
```

### Test additions in `crates/mika-cli/src/tui/app.rs`

```rust
#[test]
fn test_history_load_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let h = InputHistory::load(dir.path());
    assert!(h.entries.is_empty());
}

#[test]
fn test_history_save_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = InputHistory::load(dir.path());
    h.push("hello".to_string());
    h.push("world\nwith newlines".to_string());

    let h2 = InputHistory::load(dir.path());
    assert_eq!(h2.entries.len(), 2);
    assert_eq!(h2.entries[0], "hello");
    assert_eq!(h2.entries[1], "world\nwith newlines");
}

#[test]
fn test_history_load_corrupt_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".input_history"), "not valid json{{{").unwrap();
    let h = InputHistory::load(dir.path());
    assert!(h.entries.is_empty());
}

#[test]
fn test_history_load_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".input_history"), "").unwrap();
    let h = InputHistory::load(dir.path());
    assert!(h.entries.is_empty());
}
```

## Dependencies & Risks

- **serde_json dependency** — Already in the workspace (`Cargo.toml` line 75: `serde_json = "1"`). `serde` with `derive` feature is also available (line 74).
- **tempfile dependency** — Already in dev-dependencies for existing tests.
- **tui-textarea 0.7 `insert_str`** — Confirmed available since 0.6.0. Handles `\n` correctly. Does not handle `\r\n` natively, hence the normalization step.
- **Risk: History file bloat** — Capped at 500 entries. At worst ~500KB for very long messages. Acceptable.

## References

- `crates/mika-cli/src/tui/app.rs:68-147` — Current `InputHistory` implementation
- `crates/mika-cli/src/tui/input.rs:187-211` — Current buggy `handle_paste`
- `crates/mika-cli/src/commands/chat.rs:210-399` — TUI main loop and agent switch
- `crates/mika-cli/src/tui/input.rs:79-109` — Ctrl+V clipboard image handling (not affected)
- tui-textarea docs: `TextArea::insert_str()` — https://docs.rs/tui-textarea/0.7.0/tui_textarea/struct.TextArea.html
