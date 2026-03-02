---
title: Shell-like Slash Command Autocompletion with Argument Completion
date: 2026-03-02
category: ui-bugs
tags: [tui, autocompletion, slash-commands, input-handling, ratatui]
severity: medium
component: mika-cli
modules: [crates/mika-cli/src/tui/commands/autocomplete.rs, crates/mika-cli/src/tui/commands/completers.rs, crates/mika-cli/src/tui/commands/mod.rs, crates/mika-cli/src/tui/input.rs, crates/mika-cli/src/tui/ui.rs]
symptoms:
  - Tab acts same as Down arrow (cycles through matches instead of completing)
  - Enter in popup always executes immediately (no way to add arguments interactively)
  - No argument completion — popup dismisses on space
  - Users must type arguments from memory
root_cause: Flat AutocompleteState with visible/items/selected fields had no concept of completion modes or argument position
resolution_type: feature-enhancement
---

# Shell-like Slash Command Autocompletion

## Problem

The TUI autocompletion was rudimentary:
- Tab and Down arrow were identical (both cycled through matches)
- Enter always executed immediately, even for commands needing arguments
- Typing a space dismissed the popup entirely — no argument completion
- All 19 slash commands required typing arguments from memory

## Root Cause

The `AutocompleteState` was a flat struct (`visible: bool`, `items: Vec<&SlashCommand>`, `selected: usize`) with no concept of completion phases. The input handler treated all keystrokes uniformly with no distinction between command-level and argument-level completion.

## Solution

### 1. CompletionMode State Machine

Replaced the flat struct with a mode-aware enum:

```rust
pub enum CompletionMode {
    Hidden,
    Command { items: Vec<&'static SlashCommand>, selected: usize },
    Argument { items: Vec<CompletionItem>, selected: usize, title: &'static str, wide: bool },
}
```

This cleanly separates command-name completion from argument completion, with each mode carrying its own item list and selection state.

### 2. Bash-style Tab Completion

Tab now computes the longest common prefix (LCP) of all visible matches:
- If LCP is longer than current input → partial completion (extend to LCP)
- If LCP equals input and exactly one match → full completion + space
- If LCP equals input and multiple matches → cycle to next (visual highlight only)

### 3. Smart Enter

Enter behavior depends on the selected command's `args_hint`:
- `args_hint: None` → execute immediately (same as before for `/help`, `/clear`, etc.)
- `args_hint: Some(...)` → accept command name, append space, transition to argument mode

### 4. Argument Completers

Added `CompleterFn` type and optional `completer` field to `SlashCommand`. Each completer receives `(arg_text, arg_index, &CompletionContext)` and returns `(Vec<CompletionItem>, title)`.

Eight commands have completers:
| Command | Completions | Source |
|---------|------------|--------|
| `/model` | sonnet, opus, haiku | Static |
| `/think` | off, low, medium, high | Static |
| `/memory` | search | Static |
| `/config` | set/get → keys → values | Static + SETTABLE_CONFIG_KEYS |
| `/switch` | Agent names (excl. current) | `list_agents()` |
| `/team` | Team names | `list_teams()` |
| `/skill` | Skill names | `SkillRegistry` |
| `/attach` | File paths | `std::fs::read_dir()` |

### 5. Multi-arg Context

`CompletionContext` includes `args_str` so multi-level completers (like `/config set thinking_level <tab>`) can inspect previous arguments to offer context-appropriate values.

## Key Implementation Details

- **File path completion**: Tilde expansion restricted to `~` or `~/` (not `~otheruser`), hidden files excluded unless prefix starts with `.`, capped at 100 entries.
- **Popup width**: `wide: bool` field on Argument variant controls layout (80 for file paths, 55 for others) — avoids stringly-typed title comparison.
- **Paste handling**: After bracketed paste, re-evaluate autocomplete state by calling `update_command()` with the new input.
- **Text insertion**: Uses `textarea.insert_str()` (not TextArea reconstruction) per learnings from `tui-persistent-history-and-paste-cursor.md`.

## Gotchas

1. **`args_str` parsing**: `parse_arg_position()` uses `split_whitespace()` to count args and determine current prefix. If args end with a space, the next arg_index is `parts.len()` (starting new arg). Otherwise, the last part is the current prefix at index `parts.len() - 1`.

2. **Config value completion**: `complete_config` at arg_index 2 must check the actual config key from `ctx.args_str` before offering values. Only `thinking_level` has enumerable values — other keys return empty completions.

3. **Backspace boundary**: When backspace removes the space between command and args, the input no longer contains a space so `update_command()` naturally transitions back to command mode.

4. **Cursor positioning after `set_textarea()`**: `TextArea::from(...)` defaults cursor to (0, 0). The `set_textarea()` helper must call `move_cursor(CursorMove::End)` after construction to place the cursor at the end of the completed text. Without this, any further typing inserts at position 0 instead of after the completion.
