---
title: Fix TUI Persistent Input History and Paste Cursor Positioning
date: 2026-03-01
category: ui-bugs
tags: [tui, input-handling, history, paste, cursor-positioning, ratatui, tui-textarea]
severity: medium
component: mika-cli
modules: [crates/mika-cli/src/tui/app.rs, crates/mika-cli/src/tui/input.rs, crates/mika-cli/src/commands/chat.rs]
symptoms:
  - Input history (Up/Down arrows) lost on TUI exit
  - Paste inserts text at end of last line instead of cursor position
  - Cursor jumps to (0,0) after paste
  - Cannot undo paste (Ctrl+Z) because TextArea was reconstructed
root_cause: InputHistory was in-memory only; handle_paste() reconstructed TextArea instead of using insert_str()
resolution_type: bug-fix + feature-enhancement
---

# TUI Persistent Input History and Paste Cursor Fix

## Problem

Two TUI input handling bugs degraded the Mika CLI chat experience:

1. **History lost on exit** -- `InputHistory` was purely in-memory (`Vec<String>`). Exiting and relaunching the TUI started with empty history.
2. **Paste at wrong position** -- `handle_paste()` reconstructed the entire `TextArea` by appending pasted text to the last line, ignoring cursor position. After paste, cursor jumped to (0,0). Undo was impossible since the TextArea was recreated from scratch.

## Root Cause

### History Loss

`InputHistory::new()` created an empty vector with no file path. The `push()` method only appended to the in-memory vec. No file I/O existed.

### Paste Cursor Bug

The `handle_paste()` function manually split text into lines and reconstructed the TextArea:

```rust
// OLD (broken) -- reconstructed TextArea, lost cursor
let mut current: Vec<String> = app.textarea.lines().to_vec();
let paste_lines: Vec<&str> = text.lines().collect();
if let Some(last) = current.last_mut() {
    if let Some((first, rest)) = paste_lines.split_first() {
        last.push_str(first);  // Always appended to LAST line
        for line in rest { current.push(line.to_string()); }
    }
}
app.textarea = TextArea::from(current);  // Cursor reset to (0,0)
```

`tui-textarea` 0.7 provides `insert_str()` which correctly inserts at the cursor position -- the code bypassed this.

## Solution

### Persistent Input History

Added `file_path: Option<PathBuf>` to `InputHistory`. New constructor `load(home_dir)` reads from `{home_dir}/.input_history` (JSON array). `push()` calls `save()` after each entry.

```rust
pub fn load(home_dir: &Path) -> Self {
    let file_path = home_dir.join(HISTORY_FILENAME);
    let entries = Self::read_file(&file_path).unwrap_or_default();
    Self { entries, index: None, saved_draft: None, file_path: Some(file_path) }
}
```

**Atomic writes** prevent corruption (write to `.tmp`, rename):

```rust
#[cfg(unix)]
{
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true)
        .mode(0o600)  // Secure permissions from creation
        .open(&tmp)?
        .write_all(json.as_bytes())?;
}
std::fs::rename(&tmp, path)?;
```

**Graceful degradation**: missing file = empty history, corrupt JSON = empty history + `tracing::warn`.

**Agent switch**: history reloads when `app.home_dir` changes (`chat.rs`).

### Paste Cursor Fix

Replaced 19 lines of manual reconstruction with 1 library call:

```rust
let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
app.textarea.insert_str(&normalized);
```

Added 100KB paste size limit with user warning, and empty paste early return.

## Key Design Decisions

| Decision | Rationale |
|---|---|
| JSON file (not SQLite) | History is CLI-only state, not agent data. Avoids coupling mika-cli to mika-agent's database. |
| Per-agent history (`home_dir`) | Different agents serve different contexts; mixing history would be confusing. |
| Atomic writes (tmp + rename) | History is written on every Enter press. Crash during write must not corrupt the file. |
| `OpenOptions::mode(0o600)` | Eliminates permission race window vs. write-then-chmod. History may contain sensitive input. |
| `#[cfg(test)]` on `new()` | Prevents production code from accidentally constructing in-memory-only history. |
| 100KB paste limit | `insert_str` re-layouts the widget. Multi-MB paste would freeze the TUI. |

## Prevention Strategies

1. **Prefer library methods over manual reimplementation** -- `insert_str()` is 1 line, correct, and preserves undo. The manual approach was 19 lines and buggy.
2. **Read the full widget API** before implementing text manipulation. Check docs.rs for methods like `insert_str`, `insert_char`, `move_cursor`.
3. **Ask "what state should survive restart?"** for any interactive TUI. History persistence is the default user expectation.
4. **Always use atomic writes** for frequently-written files. Write-tmp-then-rename is trivial and prevents corruption.
5. **Set file permissions at creation time** using `OpenOptions::mode()`, not write-then-chmod.
6. **Test cursor position explicitly** after text manipulation operations.

## Files Changed

- `crates/mika-cli/src/tui/app.rs` -- `InputHistory`: added `file_path`, `load()`, `save()`, `read_file()`, `write_file()`, 6 new tests
- `crates/mika-cli/src/tui/input.rs` -- `handle_paste()`: replaced with `insert_str()`, added size limit and `\r\n` normalization
- `crates/mika-cli/src/commands/chat.rs` -- Agent switch: reload history after `home_dir` changes

## Related

- Plan: `docs/plans/2026-03-01-fix-tui-persistent-history-and-paste-cursor-plan.md`
- tui-textarea docs: https://docs.rs/tui-textarea/0.7.0/tui_textarea/struct.TextArea.html
- Precedent for 0600 permissions: `mcp.json` handling in `crates/mika-agent/src/mcp/config.rs`
