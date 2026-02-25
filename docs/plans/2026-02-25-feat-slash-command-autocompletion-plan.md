---
title: "feat: Add slash-command autocompletion to mika-cli TUI"
type: feat
status: completed
date: 2026-02-25
---

# Slash-Command Autocompletion for Mika CLI TUI

## Overview

Add a client-side slash-command system to the mika-cli chat TUI. Users type `/` in the input area to trigger an autocomplete popup showing available commands. Commands execute locally without hitting the agent loop. Tab and arrow keys cycle through completions; Enter executes. Commands are registered in a static registry so adding new ones is a single-line addition.

## Problem Statement

The mika-cli TUI currently has no in-session commands. Users must quit the TUI to run `mika status`, `mika memory`, or `mika reminders` as separate CLI subcommands. There's no way to clear the display, trigger compaction, inspect skills, or export conversations without leaving the chat. This breaks flow and makes the TUI feel limited compared to other AI chat interfaces (Claude Code, OpenClaw) that support slash commands.

## Proposed Solution

### Architecture

```
User types "/" in textarea
  → input.rs detects "/" prefix, activates AutocompleteState
  → ui.rs renders popup overlay above input area
  → Tab/Down/Up navigate popup; Esc dismisses
  → Enter executes command OR selects completion
  → Command handler dispatches to the appropriate function
  → Output rendered inline as ChatRole::Command message
  → Regular messages (no "/" prefix) go to agent as normal
```

### Data Access Strategy

Store clones of shared resources in `App`:

```rust
pub struct App<'a> {
    // ... existing fields ...
    pub db: AsyncDatabase,           // Clone (Arc-wrapped, cheap)
    pub claude: ClaudeClient,        // Clone (Arc<reqwest::Client>)
    pub home_dir: PathBuf,
    pub skills: Arc<SkillRegistry>,  // Already Arc in server/scheduler
    pub autocomplete: AutocompleteState,
}
```

This avoids new channel variants. `AsyncDatabase` and `ClaudeClient` are already designed to be cloned and shared. `SkillRegistry` is wrapped in `Arc` by the server and scheduler — extend this pattern to CLI.

### Key Design Decisions

1. **Slash commands are client-side only** — they never reach the agent loop. Unknown commands show an error message, not forwarded to the agent.
2. **Prefix matching, not fuzzy** — `/me` matches `/memory`, `/model`. Simple and predictable for a 15-command list.
3. **`/` alone shows all commands** — popup appears immediately on `/`, dismissed on Esc or backspace past `/`.
4. **Esc is two-stage** — first Esc dismisses popup (keeps typed text), second Esc clears input (existing behavior).
5. **New `ChatRole::Command` variant** — command output rendered in `Color::Cyan` to distinguish from errors (`System` = red) and conversation.
6. **Command output not stored in DB** — slash command output is display-only, not part of the conversation history the agent sees.
7. **Block destructive commands while agent is busy** — `/compact` and `/clear --all` show "Cannot run while agent is busy" if `AgentStatus != Idle`.
8. **`/config set` writes TOML, requires restart** — runtime config mutation is out of scope; simplest safe approach.
9. **`/model` is display-only for MVP** — switching models mid-session requires agent worker changes, deferred to post-MVP.

## Technical Approach

### Phase 1: Foundation (command registry + input handling)

#### Step 1: Create `SlashCommand` registry module

**File:** `crates/mika-cli/src/tui/commands/mod.rs` (new)

```rust
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: Option<&'static str>,
}

pub fn all_commands() -> &'static [SlashCommand] {
    &[
        SlashCommand { name: "help",      aliases: &["h", "?"],  description: "List available commands",           args_hint: None },
        SlashCommand { name: "clear",     aliases: &[],           description: "Clear chat display (--all for DB)", args_hint: Some("[--all]") },
        SlashCommand { name: "exit",      aliases: &["quit", "q"], description: "Quit mika",                       args_hint: None },
        SlashCommand { name: "compact",   aliases: &[],           description: "Compact conversation history",      args_hint: None },
        SlashCommand { name: "memory",    aliases: &["mem"],      description: "Show core memory blocks",           args_hint: Some("[search <query>]") },
        SlashCommand { name: "reminders", aliases: &["remind"],   description: "List active reminders",             args_hint: None },
        SlashCommand { name: "status",    aliases: &["stat"],     description: "Show system health info",           args_hint: None },
        SlashCommand { name: "soul",      aliases: &[],           description: "Display current soul.md",           args_hint: None },
        SlashCommand { name: "config",    aliases: &["cfg"],      description: "Show current config",               args_hint: Some("[set <key> <value>]") },
        SlashCommand { name: "model",     aliases: &[],           description: "Show active model",                 args_hint: None },
        SlashCommand { name: "export",    aliases: &[],           description: "Export conversation to markdown",   args_hint: None },
        SlashCommand { name: "skills",    aliases: &[],           description: "List loaded skills",                args_hint: None },
        SlashCommand { name: "skill",     aliases: &[],           description: "Show skill details",                args_hint: Some("<name>") },
    ]
}

pub fn filter_commands(prefix: &str) -> Vec<&'static SlashCommand> {
    let prefix_lower = prefix.to_lowercase();
    all_commands()
        .iter()
        .filter(|cmd| {
            cmd.name.starts_with(&prefix_lower)
                || cmd.aliases.iter().any(|a| a.starts_with(&prefix_lower))
        })
        .collect()
}
```

#### Step 2: Create `AutocompleteState`

**File:** `crates/mika-cli/src/tui/commands/autocomplete.rs` (new)

```rust
pub struct AutocompleteState {
    pub visible: bool,
    pub items: Vec<&'static SlashCommand>,
    pub selected: usize,
}

impl AutocompleteState {
    pub fn new() -> Self { Self { visible: false, items: Vec::new(), selected: 0 } }
    pub fn update(&mut self, input: &str);   // Refilter based on current input
    pub fn next(&mut self);                   // Cycle selected down
    pub fn previous(&mut self);              // Cycle selected up
    pub fn selected_name(&self) -> Option<&str>;
    pub fn dismiss(&mut self);
}
```

The `update()` method: if input starts with `/` and has no space yet, extract the prefix after `/` and call `filter_commands()`. If results are non-empty, show popup. If input doesn't start with `/` or has a space (argument mode), dismiss.

#### Step 3: Add `ChatRole::Command` variant

**File:** `crates/mika-cli/src/tui/app.rs`

```rust
pub enum ChatRole {
    User,
    Assistant,
    System,
    Command,  // new — slash command output
}
```

Update `ui.rs` rendering: `ChatRole::Command` renders with `Color::Cyan` prefix `"[cmd] "` or similar.

#### Step 4: Add shared resources to `App`

**File:** `crates/mika-cli/src/tui/app.rs`

Add fields to `App`:
```rust
pub db: AsyncDatabase,
pub claude: ClaudeClient,
pub home_dir: PathBuf,
pub skills: Arc<SkillRegistry>,
pub autocomplete: AutocompleteState,
```

Update `App::new()` signature and `chat.rs` construction site. The `AsyncDatabase` and `ClaudeClient` are cloned from the same instances passed to the agent worker.

#### Step 5: Rewrite `input.rs` key handling

**File:** `crates/mika-cli/src/tui/input.rs`

New priority order:

```rust
pub fn handle_key(app: &mut App<'_>, key: KeyEvent) {
    // 1. Ctrl+C: always quit
    // 2. If autocomplete visible:
    //    - Esc: dismiss popup (don't clear input)
    //    - Tab/Down: next suggestion
    //    - Up: previous suggestion
    //    - Enter: accept selected completion OR execute if exact match
    //    - Other keys: pass to textarea, then update autocomplete filter
    // 3. If autocomplete not visible:
    //    - Esc: clear input (existing)
    //    - PageUp/PageDown: scroll (existing)
    //    - Enter: if input starts with "/", dispatch slash command;
    //             else send_message() (existing)
    //    - Up/Down: history navigation when input empty (existing)
    //    - Tab: if input starts with "/", open autocomplete
    //    - Other: pass to textarea, check if "/" was just typed → open autocomplete
}
```

#### Step 6: Render autocomplete popup in `ui.rs`

**File:** `crates/mika-cli/src/tui/ui.rs`

After rendering the main layout, conditionally render an overlay:

```rust
if app.autocomplete.visible && !app.autocomplete.items.is_empty() {
    let popup_height = app.autocomplete.items.len().min(10) as u16 + 2; // +2 for border
    let popup_area = Rect {
        x: input_area.x + 2,  // align with input prompt
        y: input_area.y.saturating_sub(popup_height),
        width: 40.min(input_area.width),
        height: popup_height,
    };
    // Clear background, render List widget with Block border
    // Highlight selected item with Style::new().bg(Color::DarkGray)
}
```

Max 10 items visible with scroll indicator. Popup overlays the bottom of the message area.

### Phase 2: Command Handlers

#### Step 7: Create command dispatch and handlers

**File:** `crates/mika-cli/src/tui/commands/handlers.rs` (new)

```rust
pub async fn dispatch(app: &mut App<'_>, input: &str) -> Option<String> {
    let (cmd_name, args) = parse_command(input);
    match cmd_name {
        "help" | "h" | "?" => Some(handle_help()),
        "clear"             => Some(handle_clear(app, args).await),
        "exit" | "quit" | "q" => { app.should_quit = true; None }
        "compact"           => Some(handle_compact(app).await),
        "memory" | "mem"    => Some(handle_memory(app, args).await),
        "reminders" | "remind" => Some(handle_reminders(app).await),
        "status" | "stat"   => Some(handle_status(app).await),
        "soul"              => Some(handle_soul(app).await),
        "config" | "cfg"    => Some(handle_config(app, args).await),
        "model"             => Some(handle_model(app)),
        "export"            => Some(handle_export(app).await),
        "skills"            => Some(handle_skills(app)),
        "skill"             => Some(handle_skill(app, args)),
        _                   => Some(format!("Unknown command: /{}. Type /help for available commands.", cmd_name)),
    }
}

fn parse_command(input: &str) -> (&str, &str) {
    let trimmed = input.trim_start_matches('/').trim();
    match trimmed.split_once(char::is_whitespace) {
        Some((cmd, args)) => (cmd, args.trim()),
        None => (trimmed, ""),
    }
}
```

#### Step 8: Implement individual handlers

Each handler is a standalone async function. Ordered by complexity:

**Trivial (no dependencies):**
- `handle_help()` — format `all_commands()` as a table
- `handle_model(app)` — return `format!("Current model: {}", app.model)`
- `handle_exit(app)` — set `should_quit = true`

**DB read-only:**
- `handle_memory(app, args)` — if args starts with "search", call `db.search_memory(query)`; else call `db.get_all_core_memory()` and format blocks
- `handle_reminders(app)` — call `db.get_pending_reminders()` + `db.get_future_reminders()`, format as list
- `handle_status(app)` — call `db.db_size_bytes()`, `db.count_messages()`, `db.schema_version()`, format as table
- `handle_skills(app)` — iterate `app.skills.skills()`, format name + description + handler type
- `handle_skill(app, args)` — find skill by name, show full details

**File I/O:**
- `handle_soul(app)` — read `app.home_dir.join("soul.md")` via `tokio::fs::read_to_string`
- `handle_config(app, args)` — if no args, read and display config TOML; if "set key value", write updated TOML
- `handle_export(app)` — format `app.messages` as markdown, write to `~/.mika/exports/session-{id}-{date}.md`

**Agent-busy gated:**
- `handle_clear(app, args)` — clear `app.messages`; if `--all`, check `AgentStatus::Idle`, then truncate DB
- `handle_compact(app)` — check `AgentStatus::Idle`, then call `compaction::maybe_compact(&app.db, &app.claude)`

#### Step 9: Wire dispatch into `send_message()`

**File:** `crates/mika-cli/src/tui/app.rs`

```rust
pub fn send_message(&mut self) {
    let text = self.textarea.lines().join("\n").trim().to_string();
    if text.is_empty() { return; }

    self.autocomplete.dismiss();

    if text.starts_with('/') {
        // Don't add to input history (command output is ephemeral)
        self.textarea = TextArea::default(); // reset
        // Spawn async handler — can't await in sync context
        // Use a oneshot channel or push to a command queue
        self.pending_command = Some(text);
        return;
    }

    // ... existing agent send logic ...
}
```

Since `send_message()` is called from the sync `handle_key` context but handlers are async, use a `pending_command: Option<String>` field. Process it in `tick()`:

```rust
pub async fn tick(&mut self) {
    // Process pending slash command
    if let Some(cmd) = self.pending_command.take() {
        if let Some(output) = commands::dispatch(self, &cmd).await {
            self.messages.push(ChatMessage {
                role: ChatRole::Command,
                content: output,
                rendered: None,
            });
        }
    }
    // ... existing tick logic ...
}
```

**Note:** This requires `tick()` to become `async`. Currently it is sync. The main loop in `chat.rs` already runs inside a tokio runtime, so converting `tick()` to async is straightforward — change `app.tick()` to `app.tick().await` in the event loop.

### Phase 3: Polish

#### Step 10: Update footer hint

**File:** `crates/mika-cli/src/tui/ui.rs`

Add `"/ commands"` to the footer alongside `"Ctrl+C quit"`:

```rust
let hints = "/ commands | Ctrl+C quit";
```

#### Step 11: Add tests

**File:** `crates/mika-cli/src/tui/commands/mod.rs` (inline tests)

```rust
#[cfg(test)]
mod tests {
    #[test] fn test_filter_exact_match() { ... }
    #[test] fn test_filter_prefix() { ... }
    #[test] fn test_filter_alias() { ... }
    #[test] fn test_filter_no_match() { ... }
    #[test] fn test_parse_command_no_args() { ... }
    #[test] fn test_parse_command_with_args() { ... }
    #[test] fn test_parse_command_extra_whitespace() { ... }
}
```

**File:** `crates/mika-cli/src/tui/commands/autocomplete.rs` (inline tests)

```rust
#[cfg(test)]
mod tests {
    #[test] fn test_update_shows_all_on_slash() { ... }
    #[test] fn test_update_filters_on_prefix() { ... }
    #[test] fn test_update_hides_on_no_match() { ... }
    #[test] fn test_next_wraps_around() { ... }
    #[test] fn test_dismiss_resets_state() { ... }
}
```

**File:** `crates/mika-cli/src/tui/commands/handlers.rs` (inline tests)

```rust
#[cfg(test)]
mod tests {
    #[test] fn test_dispatch_help() { ... }
    #[test] fn test_dispatch_unknown() { ... }
    #[test] fn test_parse_command_variations() { ... }
}
```

#### Step 12: Update `chat.rs` to pass resources

**File:** `crates/mika-cli/src/commands/chat.rs`

```rust
let app = App::new(
    user_tx,
    agent_rx,
    session_id,
    model,
    identity_name,
    db.clone(),           // new
    claude.clone(),       // new
    home_dir.clone(),     // new
    Arc::new(skills),     // new — wrap in Arc before passing to both App and worker
);
```

## Acceptance Criteria

### Functional Requirements

- [ ] Typing `/` in the input area shows a popup with all available commands
- [ ] Typing additional characters filters the popup (prefix match)
- [ ] Tab and Down arrow cycle forward through popup items
- [ ] Up arrow cycles backward through popup items
- [ ] Enter executes the selected/typed command
- [ ] Esc dismisses the popup without clearing input
- [ ] Each MVP slash command executes and displays output inline
- [ ] Unknown commands show "Unknown command" message
- [ ] Regular messages (no `/` prefix) still go to the agent
- [ ] `/compact` and `/clear --all` are blocked when agent is busy
- [ ] Command output is visually distinct from errors and conversation

### Non-Functional Requirements

- [ ] No new crate dependencies (ratatui + tui-textarea + crossterm are sufficient)
- [ ] Adding a new command requires only: add to `all_commands()` + add match arm in `dispatch()`
- [ ] All 15 MVP commands are implemented and tested
- [ ] Popup renders correctly on terminals >= 80x24
- [ ] No performance regression in the event loop (popup filtering is O(n) with n=15)

### Quality Gates

- [ ] `cargo test` passes (existing + new tests)
- [ ] `cargo clippy` clean
- [ ] `cargo fmt` clean
- [ ] Manual testing of all 15 commands in the TUI

## File Inventory

### New Files

| File | Purpose | Est. Lines |
|------|---------|------------|
| `crates/mika-cli/src/tui/commands/mod.rs` | SlashCommand registry, filter, parse | ~80 |
| `crates/mika-cli/src/tui/commands/autocomplete.rs` | AutocompleteState management | ~80 |
| `crates/mika-cli/src/tui/commands/handlers.rs` | Command dispatch + individual handlers | ~350 |

### Modified Files

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/app.rs` | Add `ChatRole::Command`, `AutocompleteState`, `db`, `claude`, `home_dir`, `skills`, `pending_command` fields; make `tick()` async |
| `crates/mika-cli/src/tui/input.rs` | Rewrite key handling with autocomplete priority; intercept Tab, Enter for `/` prefix |
| `crates/mika-cli/src/tui/ui.rs` | Add popup overlay rendering; add `ChatRole::Command` color; update footer hints |
| `crates/mika-cli/src/tui/mod.rs` | Add `pub mod commands;` |
| `crates/mika-cli/src/commands/chat.rs` | Pass `db`, `claude`, `home_dir`, `skills` to App; make `tick()` call async |

### Unchanged (reference only)

| File | Why Referenced |
|------|---------------|
| `crates/mika-agent/src/async_db.rs` | DB methods called by handlers |
| `crates/mika-agent/src/compaction.rs` | `maybe_compact()` called by /compact |
| `crates/mika-agent/src/skills/mod.rs` | `SkillRegistry::skills()` called by /skills |
| `crates/mika-common/src/home.rs` | `home_dir` for /soul, /config, /export |

## Dependencies & Risks

### Dependencies
- `AsyncDatabase` must remain `Clone` (currently is — Arc-wrapped)
- `ClaudeClient` must remain `Clone` (currently is — Arc<reqwest::Client>)
- `SkillRegistry` must be wrapped in `Arc` for sharing (already done in server/scheduler)

### Risks
1. **`tick()` becoming async** — requires changing the main event loop in `chat.rs`. The loop already runs inside a tokio runtime, so this should be straightforward, but it changes the call site.
2. **Concurrent `/compact` + agent compaction** — mitigated by blocking when agent is busy.
3. **`tui-textarea` key event consumption** — Tab and arrow keys must be intercepted before reaching the textarea widget. Testing needed to ensure no double-handling.

## Post-MVP Enhancements (Not in Scope)

- `/model <name>` live switching (requires `AgentRequest::SetModel` variant)
- `/config set` runtime mutation (requires `Arc<RwLock<Settings>>`)
- `/reminders cancel <id>` sub-command
- `/memory reset <block>` sub-command
- Fuzzy matching for command names
- Second-level argument autocompletion (e.g., `/config set <tab>` showing config keys)
- Persistent command history separate from message history

## References

### Internal
- `crates/mika-cli/src/tui/input.rs` — current key handling (54 lines)
- `crates/mika-cli/src/tui/app.rs` — App struct, send_message(), tick()
- `crates/mika-cli/src/tui/ui.rs` — layout and rendering
- `crates/mika-cli/src/commands/chat.rs` — agent worker spawn, main event loop
- `crates/mika-cli/src/cli.rs` — existing clap subcommands (mirror surface area)
- `docs/solutions/code-review-workflow/mika-cli-21-findings-parallel-resolution.md` — TUI learnings (UTF-8 safety, cached rendering, scroll offset)

### External (OpenClaw patterns studied)
- `openclaw/src/tui/commands.ts` — SlashCommand type with `getArgumentCompletions` callback
- `openclaw/src/auto-reply/reply/commands-slash-parse.ts` — structured parse result enum
- `openclaw/src/tui/tui.ts` — CombinedAutocompleteProvider, editor submit routing
- Key takeaway: OpenClaw routes unknown commands to the agent; we reject them locally instead (simpler for MVP).
