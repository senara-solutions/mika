---
title: "feat: Shell-like slash command autocompletion with argument completion"
type: feat
status: completed
date: 2026-03-02
---

# Shell-like Slash Command Autocompletion

## Overview

Upgrade the TUI slash command autocompletion from simple tab-cycling to shell-like behavior with prefix completion, arrow key navigation, and context-aware argument completion. Currently, pressing `/` shows commands and Tab/arrows cycle through them, but the popup dismisses on space and no argument completion exists.

## Problem Statement / Motivation

The current autocompletion is rudimentary:
- Tab acts the same as Down arrow (cycles through matches)
- Enter in the popup executes immediately (no way to add arguments interactively)
- No argument completion at all — the popup dismisses when a space is typed
- Users familiar with shell completion (bash/zsh/fish) expect Tab to complete, not cycle

Commands like `/model`, `/switch`, `/team`, `/skill`, `/attach`, and `/config set` all take arguments that could be completed from known data sources, but users must type them from memory.

## Design Decisions

Based on SpecFlow analysis, these foundational decisions are adopted:

1. **Tab behavior**: Bash-style — Tab inserts longest common prefix; if only one match, completes fully and appends a space. Subsequent Tabs with no prefix progress cycle through matches.
2. **Enter in popup**: Smart — commands with `args_hint: None` execute immediately; commands with `args_hint: Some(...)` accept the command name, append a space, and transition to argument mode.
3. **Argument completion trigger**: Lazy (Tab-triggered), consistent with shell behavior. No eager popup on space.
4. **File path root**: Current working directory of the mika process. `~` expands to `$HOME`.
5. **Filesystem reads**: Synchronous but fast (local SQLite-backed dirs are tiny). Pre-cache agent/team lists at startup, refresh on mutation events. Use `spawn_blocking` only for `/attach` file path traversal.
6. **Current agent filtering**: `/switch` completions exclude the current agent.
7. **Large value domains**: `/config set timezone` uses type-ahead prefix filtering (not full list). `/config set chat_id` offers no completions.
8. **Popup title**: Changes contextually — " Commands ", " Models ", " Agents ", " Teams ", " Skills ", " Files ", " Config Keys ".

## Proposed Solution

### New Data Model

Replace the flat `AutocompleteState` with a mode-aware state machine:

```rust
// crates/mika-cli/src/tui/commands/autocomplete.rs

/// A single completion candidate
pub struct CompletionItem {
    /// The value to insert (e.g., "sonnet", "main", "~/Documents")
    pub value: String,
    /// Optional description shown alongside (e.g., "Claude Sonnet 4.6")
    pub description: Option<String>,
}

/// What kind of completion is active
pub enum CompletionMode {
    /// No popup visible
    Hidden,
    /// Completing a command name after "/"
    Command {
        items: Vec<CompletionItem>,
        selected: usize,
    },
    /// Completing an argument for a known command
    Argument {
        command_name: &'static str,
        arg_index: usize,
        items: Vec<CompletionItem>,
        selected: usize,
    },
}

pub struct AutocompleteState {
    pub mode: CompletionMode,
}
```

### Argument Completer on SlashCommand

Extend the `SlashCommand` struct with an optional completer:

```rust
// crates/mika-cli/src/tui/commands/mod.rs

pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: Option<&'static str>,
    /// Returns completion candidates for the given argument prefix and position.
    /// `arg_text` is the current argument being typed, `arg_index` is which
    /// argument (0-based), `ctx` provides access to app state.
    pub completer: Option<fn(arg_text: &str, arg_index: usize, ctx: &CompletionContext) -> Vec<CompletionItem>>,
}
```

Where `CompletionContext` exposes the minimal state needed:

```rust
// crates/mika-cli/src/tui/commands/mod.rs

pub struct CompletionContext<'a> {
    pub home_dir: &'a Path,        // Per-agent home (~/.mika/agents/main/)
    pub global_home: &'a Path,     // Global home (~/.mika/)
    pub skills: &'a SkillRegistry,
    pub current_agent: &'a str,
    pub cwd: &'a Path,            // For file path completion
}
```

### Completer Implementations by Command

| Command | arg_index=0 | arg_index=1 | Source |
|---------|-------------|-------------|--------|
| `/model` | Model aliases: sonnet, opus, haiku | — | Static (`MODEL_ALIASES`) |
| `/think` | Levels: off, low, medium, high | — | Static |
| `/switch` | Agent names (excl. current) | — | `list_agents(global_home)` |
| `/agent` | Agent names | — | `list_agents(global_home)` |
| `/team` | Team names | — | `list_teams(home_dir)` |
| `/skill` | Skill names | — | `skills.skills()` |
| `/attach` | File paths | — | `std::fs::read_dir(cwd)` |
| `/config` | Subcommands: set, get | Config keys (for "set") | Static + `CONFIG_KEYS` |
| `/memory` | Subcommands: search | — | Static |
| `/export` | Formats: markdown, json, text | — | Static |

### Key Behavioral Changes

**Tab in command mode:**
1. Compute longest common prefix of all visible items
2. If prefix is longer than current input → insert prefix (partial completion)
3. If prefix equals current input AND exactly one match → complete fully, append space, transition to argument mode (if command has args) or execute (if no args)
4. If prefix equals current input AND multiple matches → cycle to next item (visual highlight only, no text insertion)

**Enter in command popup:**
- `args_hint: None` → accept command, execute immediately
- `args_hint: Some(...)` → accept command, append space, enter argument mode

**Tab in argument mode:**
1. Call the command's completer with current arg prefix and arg_index
2. Apply same common-prefix logic as command mode
3. If completed fully and no further args expected → append space (ready to execute with Enter)

**Backspace across boundary:**
- If backspace removes the space between command and args → transition from argument mode back to command mode
- If backspace removes the `/` → dismiss popup entirely

**Escape:** Always dismisses popup and returns to normal input mode.

### Rendering Changes

Modify `draw_autocomplete()` in `ui.rs` to:
1. Accept the `CompletionMode` enum instead of `Vec<&SlashCommand>`
2. Render `CompletionItem` with value + optional description
3. Change popup title based on mode (contextual titles)
4. Adapt popup width: min 30, max 60 for commands; max 80 for file paths
5. Show scroll indicator when items exceed 10

### File Path Completion Detail

```rust
fn complete_file_path(prefix: &str, cwd: &Path) -> Vec<CompletionItem> {
    let expanded = expand_tilde(prefix);
    let (dir, file_prefix) = split_path(&expanded, cwd);

    // Read directory entries, filter by prefix, sort
    let entries = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().unwrap_or("").starts_with(&file_prefix))
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = e.file_name().to_string_lossy().to_string();
            let display = if is_dir { format!("{name}/") } else { name.clone() };
            CompletionItem { value: display, description: None }
        })
        .collect();

    entries
}
```

- Limit to 100 entries to prevent slow rendering on huge directories
- Directories shown with trailing `/`
- Hidden files (`.` prefix) excluded unless the typed prefix starts with `.`
- Symlinks followed for type detection, errors silently ignored

## Technical Considerations

- **Performance**: Static completions are instant. Filesystem completions (`list_agents`, `list_teams`) scan tiny directories (typically <10 entries) — no caching needed. File path completion for `/attach` may hit large directories — capped at 100 entries with prefix filtering.
- **Thread safety**: All completion logic runs synchronously in the input handler. No async needed for the small directory reads. For `/attach` on potentially slow filesystems, wrap in `spawn_blocking` if latency is observed (deferred optimization).
- **tui-textarea interaction**: Use `textarea.insert_str()` for inserting completions (per learnings from `docs/solutions/ui-bugs/tui-persistent-history-and-paste-cursor.md`). Never reconstruct the textarea — preserves cursor position and undo stack.
- **Paste handling**: After bracketed paste, re-evaluate autocomplete state by calling `update()` with the new input content.

## Acceptance Criteria

### Functional Requirements

- [x] Typing `/` shows all 19 commands in a popup above the input
- [x] Typing further characters filters commands by prefix (name and aliases)
- [x] Arrow keys (Up/Down) navigate the popup with visual highlight
- [x] Tab completes the longest common prefix; if unique match, completes fully
- [x] Enter on a command with no args executes immediately
- [x] Enter on a command with args accepts the name, appends space, enters argument mode
- [x] Escape dismisses the popup in both command and argument modes
- [x] Tab after `/model ` shows model completions: sonnet, opus, haiku
- [x] Tab after `/think ` shows level completions: off, low, medium, high
- [x] Tab after `/switch ` shows agent names (excluding current agent)
- [x] Tab after `/team ` shows team names
- [x] Tab after `/skill ` shows skill names from the registry
- [x] Tab after `/attach ` shows file/directory entries from cwd
- [x] Tab after `/config set ` shows config keys
- [x] Tab after `/config set thinking_level ` shows valid values
- [x] Backspace from argument area back to command name transitions to command mode
- [x] Popup title changes contextually (Commands, Models, Agents, etc.)
- [x] Typing during argument completion filters the argument list in real-time

### Non-Functional Requirements

- [x] Completion lookup completes in <10ms for all static and dynamic sources
- [x] File path completion capped at 100 entries
- [x] No UI stutter or blocking during completion
- [x] All existing autocomplete tests continue to pass

### Quality Gates

- [x] Unit tests for `AutocompleteState` mode transitions
- [x] Unit tests for each completer function (model, agent, team, skill, file path, config)
- [x] Unit tests for longest-common-prefix computation
- [x] Unit tests for backspace boundary transitions
- [x] Integration test: Tab completion round-trip for `/model sonnet`
- [x] Manual test: visual popup positioning on small terminals (24x80)

## Implementation Phases

### Phase 1: Refactor AutocompleteState (Foundation)

**Files:**
- `crates/mika-cli/src/tui/commands/autocomplete.rs` — New `CompletionMode` enum, `CompletionItem` struct, refactored state machine
- `crates/mika-cli/src/tui/commands/mod.rs` — Add `completer` field to `SlashCommand`, `CompletionContext` struct
- `crates/mika-cli/src/tui/input.rs` — Update `handle_key_autocomplete` for new Tab/Enter semantics
- `crates/mika-cli/src/tui/ui.rs` — Update `draw_autocomplete` to render `CompletionItem`
- `crates/mika-cli/src/tui/app.rs` — Update `App` to construct `CompletionContext`

**Deliverables:**
- `CompletionMode` enum replaces `visible: bool` + `Vec<&SlashCommand>`
- Tab does longest-common-prefix completion for commands
- Enter is smart (execute argless, transition for arg commands)
- Popup renders `CompletionItem` with value + description
- All existing tests updated and passing

### Phase 2: Static Argument Completions

**Files:**
- `crates/mika-cli/src/tui/commands/completers.rs` (new) — Completer functions for each command
- `crates/mika-cli/src/tui/commands/mod.rs` — Wire completers into `COMMANDS` entries

**Deliverables:**
- `/model <tab>` completes model aliases with descriptions
- `/think <tab>` completes thinking levels
- `/export <tab>` completes format options
- `/memory <tab>` completes subcommands
- `/config <tab>` completes subcommands; `/config set <tab>` completes config keys; `/config set thinking_level <tab>` completes values
- Contextual popup titles

### Phase 3: Dynamic Argument Completions

**Files:**
- `crates/mika-cli/src/tui/commands/completers.rs` — Add filesystem-backed completers

**Deliverables:**
- `/switch <tab>` and `/agent <tab>` complete agent names (excluding current for /switch)
- `/team <tab>` completes team names
- `/skill <tab>` completes skill names from registry

### Phase 4: File Path Completion

**Files:**
- `crates/mika-cli/src/tui/commands/completers.rs` — File path completer with tilde expansion

**Deliverables:**
- `/attach <tab>` completes file paths from cwd
- Tilde expansion (`~` → `$HOME`)
- Directories shown with trailing `/`
- Hidden files excluded unless prefix starts with `.`
- Capped at 100 entries

### Phase 5: Polish and Edge Cases

**Deliverables:**
- Paste handling: re-evaluate autocomplete after bracketed paste
- Popup width adapts to content type (wider for file paths)
- Scroll indicator for lists >10 items
- Alias indicator in popup when matched via alias

## Dependencies & Risks

- **tui-textarea API**: `insert_str()` is the recommended method for text insertion. If the API changes in future versions, completion insertion would need updating.
- **Filesystem latency**: Directory reads for agents/teams are local and fast. `/attach` path completion on networked filesystems could be slow — mitigated by the 100-entry cap.
- **Breaking change**: Enter behavior changes from "execute immediately" to "smart dispatch." Users accustomed to the old behavior may need adjustment. Argless commands still execute immediately, minimizing disruption.

## References & Research

### Internal References
- Current autocomplete state: `crates/mika-cli/src/tui/commands/autocomplete.rs:4-63`
- Current input handler: `crates/mika-cli/src/tui/input.rs:21-185`
- Current popup renderer: `crates/mika-cli/src/tui/ui.rs:484-526`
- SlashCommand registry: `crates/mika-cli/src/tui/commands/mod.rs:5-150`
- Model aliases: `crates/mika-cli/src/tui/commands/handlers.rs:317-321`
- Agent discovery: `crates/mika-common/src/agent.rs:44-71`
- Team discovery: `crates/mika-common/src/team.rs:84-105`
- Skill registry: `crates/mika-agent/src/skills/mod.rs:19-44`
- Config keys: `crates/mika-agent/src/config_keys.rs:7`
- tui-textarea cursor fix learnings: `docs/solutions/ui-bugs/tui-persistent-history-and-paste-cursor.md`

### External References
- OpenClaw argument completion pattern: `../openclaw/src/tui/commands.ts:27-37`
- tui-textarea 0.7 API: https://docs.rs/tui-textarea/0.7.0/tui_textarea/struct.TextArea.html
