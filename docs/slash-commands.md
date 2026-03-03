# Slash Commands Reference

## Overview

Slash commands are client-side actions executed directly in the Mika TUI. They are
**never sent to the agent** -- typing `/status` runs local code that queries the
database and renders output in-place, while typing a regular message (no `/` prefix)
sends it to the Claude-backed agent loop as usual.

All 20 commands are defined in a single `COMMANDS` array
(`crates/mika-cli/src/tui/commands/mod.rs`), dispatched through pattern matching in
`handlers.rs`, and surfaced via an autocomplete popup driven by `autocomplete.rs`.

Unknown commands produce an inline error:

```
Unknown command: /foo. Type /help for available commands.
```

## Autocomplete

The TUI provides shell-like autocompletion for both slash command names and their
arguments.

### Command Completion

1. Type `/` -- a popup appears listing all 19 commands.
2. Continue typing to narrow the list. For example, `/me` narrows to `/memory`.
   Matching works on both command names and aliases (e.g., typing `/q` matches
   `/exit` via its `q` alias).

### Argument Completion

Commands that accept arguments support Tab-triggered argument completion. After
accepting a command name, press Tab to see available completions:

| Command | Completes | Source |
|---------|-----------|--------|
| `/model` | Model aliases (sonnet, opus, haiku) | Static |
| `/think` | Thinking levels (off, low, medium, high) | Static |
| `/memory` | Subcommands (search) | Static |
| `/config` | Subcommands (set, get), then config keys, then values | Static + config registry |
| `/switch` | Agent names (excluding current agent) | `~/.mika/agents/` |
| `/team` | Team names | `~/.mika/teams/` |
| `/skill` | Skill names from the registry | Skill registry |
| `/attach` | File paths with tilde expansion | Current working directory |

Argument completion is lazy (Tab-triggered) -- the popup does not appear
automatically when a space is typed after the command name.

### Tab Behavior (Bash-style)

Tab uses longest-common-prefix completion, similar to bash:

1. If multiple matches share a common prefix longer than what is typed, Tab
   extends the input to that prefix (partial completion).
2. If exactly one match remains, Tab completes it fully and appends a space.
   For argless commands, this also executes the command.
3. If multiple matches share no further common prefix, Tab cycles to the next
   suggestion.

### Enter Behavior (Smart Dispatch)

Enter is context-aware in the autocomplete popup:

- **Commands with no arguments** (e.g., `/help`, `/clear`, `/exit`): Enter
  accepts and executes immediately.
- **Commands with arguments** (e.g., `/model`, `/switch`): Enter accepts the
  command name, appends a space, and dismisses the popup so you can type or
  Tab-complete arguments.

### Popup Controls

| Key        | Action                                          |
|------------|-------------------------------------------------|
| Tab        | Longest common prefix completion (see above)    |
| Down       | Next suggestion (wraps around)                  |
| Up         | Previous suggestion (wraps around)              |
| Enter      | Accept selection (execute or transition to args) |
| Esc        | Dismiss popup (keeps typed text in input)        |
| Any other  | Passes to input field; popup re-filters          |

The popup title changes contextually: " Commands ", " Models ", " Agents ",
" Teams ", " Skills ", " Files ", " Config ", " Config Keys ", " Values ",
" Think ", " Memory ".

## Command Reference

### /help

List all available commands with their aliases, argument hints, and descriptions.

**Aliases:** `/h`, `/?` | **Arguments:** None

Example: `Available commands: /help (h, ?) -- List available commands ...`

---

### /clear

Clear all messages from the chat display and reset the scroll position.

**Aliases:** None | **Arguments:** None

Empties the in-memory message list and resets `scroll_offset` to 0. The underlying database conversation is not affected. A `--all` flag to also clear the database is planned but not yet implemented.

---

### /exit

Quit the Mika TUI. Sets `should_quit = true` and produces no output.

**Aliases:** `/quit`, `/q` | **Arguments:** None

---

### /compact

Manually trigger conversation compaction. Summarizes and trims history to reduce token usage.

**Aliases:** None | **Arguments:** None

Requires the agent to be idle and the conversation to have more than 50 messages. Outputs `Compacted conversation (N messages).`, `Nothing to compact (N/50 messages).`, or `Cannot compact while agent is busy.`

---

### /memory

Show core memory blocks or search across all structured memory layers.

**Aliases:** `/mem` | **Arguments:** `[search <query>]` (optional)

**Without arguments** -- displays all core memory entries with keys, values, and token counts, plus total usage against the 2000-token limit.

**With `search <query>`** -- searches across all Layer 2 tables (People, Commitments, Preferences, Events) concurrently and displays results grouped by category.

```
/memory search meeting
→ Commitments: [active] Review Q4 roadmap with team (due: 2026-03-01)
→ Events: Board meeting with investors (2026-03-15)
```

---

### /reminders

List all active reminders, split into pending (past-due) and upcoming (future).

**Aliases:** `/remind` | **Arguments:** None

```
Pending reminders:
  #3: Follow up with Alice on contract (due: 2026-02-24T10:00:00Z)
Upcoming reminders:
  #5: Weekly team standup (fires: 2026-02-26T09:00:00Z)
```

---

### /status

Show system health information: message count, database size, core memory usage, schema version, active model, and session ID. All four DB queries run concurrently via `tokio::join!`.

**Aliases:** `/stat` | **Arguments:** None

```
Status: Messages: 142 | DB size: 384 KB | Core memory: 52/2000 tokens | Schema: v6 | Model: claude-sonnet-4-6 | Session: a1b2c3d4
```

---

### /soul

Display the contents of `~/.mika/soul.md` (personality and behavioral guidance injected into the system prompt).

**Aliases:** None | **Arguments:** None

Shows the file contents, or `soul.md is empty.` / `No soul.md found. Create one at ~/.mika/soul.md` as appropriate.

---

### /config

Show the current configuration summary (key settings and config file path). Does not dump file contents (which may contain secrets).

**Aliases:** `/cfg` | **Arguments:** None

```
Configuration: Model: claude-sonnet-4-6 | Home: /home/sami/.mika | Session: a1b2c3d4 | Config file: /home/sami/.mika/config/local.toml
```

---

### /model

Show or switch the currently active Claude model. Without arguments, displays the
current model. With an argument, switches to that model immediately.

**Aliases:** None | **Arguments:** `[sonnet|opus|haiku]` (optional)

**Show current model:**
```
/model
→ Current model: claude-sonnet-4-6
```

**Switch model:**
```
/model opus
→ Switched to claude-opus-4-6.
```

Recognized shortcuts: `sonnet` = `claude-sonnet-4-6`, `opus` = `claude-opus-4-6`,
`haiku` = `claude-haiku-4-5`. Full model IDs (e.g., `claude-sonnet-4-6`) also work.

---

### /export

Export the current conversation to a Markdown file in `~/.mika/exports/`.

**Aliases:** None | **Arguments:** None

Creates the `exports/` directory if needed. Generates a filename using session ID prefix and UTC timestamp (e.g., `session-a1b2c3d4-2026-02-25-143022.md`). Writes User, Assistant, and System messages; command output is excluded as ephemeral. Uses `create_new` to prevent overwrites (symlink-attack safe). Output: `Exported to /home/sami/.mika/exports/session-a1b2c3d4-2026-02-25-143022.md` or `Nothing to export.`

---

### /skills

List all loaded skills from the filesystem-based skill registry.

**Aliases:** None | **Arguments:** None

Each skill shows its name, handler type (`builtin`, `exec`, or `http`), description, and whether it is always-on.

```
Loaded skills:
  web_search (builtin) — Search the web for current information
  calendar (exec) — Manage calendar events and scheduling [always on]
```

---

### /skill

Show detailed information about a specific loaded skill. Lookup is case-insensitive.

**Aliases:** None | **Arguments:** `<name>` (required)

```
Skill: web_search
  Description: Search the web for current information
  Handler: builtin (tools: web_search, browse_url)
  Always on: false | Keywords: search, look up, find online
  Path: /home/sami/.mika/skills/web_search
```

Errors: `Usage: /skill <name>` or `No skill found with name 'foo'. Use /skills to list all loaded skills.`

---

### /switch

Switch to a different agent by name. Tears down the current agent worker and
initializes a new one. Conversation history is preserved across switches.

**Aliases:** `/agent` | **Arguments:** `<name>` (required)

```
/switch work
→ Switched to agent 'work' (claude-sonnet-4-6).
```

Errors: `Usage: /switch <agent_name>` or `Failed to switch agent: ...`

---

### /agents

List all available agents (subdirectories of `~/.mika/agents/`).

**Aliases:** None | **Arguments:** None

---

### /teams

List all available team workflows (subdirectories of `~/.mika/teams/`).

**Aliases:** None | **Arguments:** None

---

### /team

Run a team workflow with a specified goal. Dispatches to the team engine for
multi-agent orchestrated execution.

**Aliases:** None | **Arguments:** `<name> "<goal>"` (required)

---

### /think

Set the extended thinking level for the current session. Without arguments, shows
the current level. With `off`, disables thinking. With a level and optional prompt,
either sets a persistent level or triggers a single thinking turn.

**Aliases:** `/t` | **Arguments:** `[low|medium|high|off] [prompt]` (optional)

**Show current level:**
```
/think
→ Thinking: off
```

**Set persistent level:**
```
/think high
→ Thinking set to high (16000 tokens). All future messages will use this level.
```

**Think once with a prompt:**
```
/think medium What is the meaning of life?
```
This sends the prompt with the specified thinking level but does not change the
persistent setting.

**Disable thinking:**
```
/think off
→ Thinking disabled.
```

Budget tokens by level: `low` = 4000, `medium` = 10000, `high` = 16000.

---

### /attach

Attach an image file to the next message. Supports PNG, JPG, GIF, and WEBP formats
up to 10MB. Magic bytes are validated against the file extension.

**Aliases:** `/img` | **Arguments:** `<path>` (required)

```
/attach ~/photos/screenshot.png
→ Attached: screenshot.png (245KB)
```

Errors: `Usage: /attach <file_path>` or `Failed to load image: ...`

Images can also be pasted from the clipboard via Ctrl+V. The TUI tries arboard
first, then falls back to xclip/wl-paste on Linux. If the clipboard has no image,
a system message suggests using `/attach` instead.

### /verbose

Toggle verbose mode in team TUI. When enabled, individual agent responses are
displayed as system messages during team execution.

**Aliases:** `/v` | **Arguments:** None

```
/verbose
→ Verbose mode: ON — showing individual agent responses.
/verbose
→ Verbose mode: OFF — showing only progress and deliverables.
```

Only available in team mode (`mika --team <name>`).

---

## Team Mode

When the TUI is launched with `mika --team <name>`, slash commands are restricted
to a safe subset. Agent-specific commands are disabled:

| Available | Disabled |
|-----------|----------|
| `/help`, `/clear`, `/exit`, `/quit` | `/model`, `/think`, `/agent`, `/switch` |
| `/export`, `/teams`, `/agents` | `/memory`, `/reminders`, `/compact`, `/soul` |
| `/status`, `/team`, `/verbose` | `/config`, `/skills`, `/skill`, `/attach` |

In team mode, `/status` and `/team` both show team info (name, orchestrator,
agents). `/export` writes to the team directory instead of the agent home.
`/verbose` toggles display of individual agent messages during team execution.

Team runs are persisted to a per-team SQLite database (`{team_dir}/data/mika.db`)
with graph-structured messages linked via `parent_id`.

## Keyboard Shortcuts

Complete key bindings for the Mika TUI. These apply regardless of slash commands.

### Global

| Key    | Action |
|--------|--------|
| Ctrl+C | Quit immediately |

### Normal Mode (autocomplete not visible)

| Key         | Action                                                |
|-------------|-------------------------------------------------------|
| Enter       | Send message to agent, or execute slash command        |
| Shift+Enter | Insert newline in input (multi-line editing)           |
| Esc         | Clear input field and reset input history index        |
| Tab         | Open autocomplete popup (when input starts with `/`). In argument territory, triggers argument completion. |
| PageUp      | Scroll message history up (5 lines)                    |
| PageDown    | Scroll message history down (5 lines)                  |
| Up          | Previous input history entry (when input is empty)     |
| Down        | Next input history entry (when input is empty)         |
| Ctrl+V      | Paste image from clipboard (falls back to text paste)  |

Note: Enter and slash commands only execute when the agent status is Idle. If the
agent is busy processing a turn, Enter has no effect.

### Autocomplete Mode (popup visible)

| Key        | Action                                          |
|------------|-------------------------------------------------|
| Tab        | Longest common prefix completion; cycles if no further prefix |
| Down       | Next suggestion (wraps around)                  |
| Up         | Previous suggestion (wraps around)              |
| Enter      | Accept selection (execute argless commands, or transition to argument input for commands with args) |
| Esc        | Dismiss popup (keeps typed text)                 |
| Any other  | Passes to input field; popup re-filters          |
