# Slash Commands Reference

## Overview

Slash commands are client-side actions executed directly in the Mika TUI. They are
**never sent to the agent** -- typing `/status` runs local code that queries the
database and renders output in-place, while typing a regular message (no `/` prefix)
sends it to the Claude-backed agent loop as usual.

All 13 commands are defined in a single `COMMANDS` array
(`crates/mika-cli/src/tui/commands/mod.rs`), dispatched through pattern matching in
`handlers.rs`, and surfaced via an autocomplete popup driven by `autocomplete.rs`.

Unknown commands produce an inline error:

```
Unknown command: /foo. Type /help for available commands.
```

## Autocomplete

The TUI provides a filtered autocomplete popup for slash commands.

**How it works:**

1. Type `/` -- a popup appears listing all 13 commands.
2. Continue typing to narrow the list. For example, `/me` narrows to `/memory`.
   Matching works on both command names and aliases (e.g., typing `/q` matches
   `/exit` via its `q` alias).
3. The popup disappears automatically once a space is typed after the command name
   (arguments are not autocompleted).

**Popup controls:**

| Key        | Action                                          |
|------------|-------------------------------------------------|
| Tab / Down | Cycle to next suggestion                        |
| Up         | Cycle to previous suggestion                    |
| Enter      | Accept selected command and execute immediately  |
| Esc        | Dismiss popup (keeps typed text in input)        |
| Any other  | Continues filtering the suggestion list          |

Selection wraps around in both directions.

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

Show the currently active Claude model.

**Aliases:** None | **Arguments:** None

Output: `Current model: claude-sonnet-4-6`

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
| Tab         | Open autocomplete popup (when input starts with `/`)   |
| PageUp      | Scroll message history up (5 lines)                    |
| PageDown    | Scroll message history down (5 lines)                  |
| Up          | Previous input history entry (when input is empty)     |
| Down        | Next input history entry (when input is empty)         |

Note: Enter and slash commands only execute when the agent status is Idle. If the
agent is busy processing a turn, Enter has no effect.

### Autocomplete Mode (popup visible)

| Key        | Action                                          |
|------------|-------------------------------------------------|
| Tab / Down | Next suggestion (wraps around)                  |
| Up         | Previous suggestion (wraps around)              |
| Enter      | Accept and execute selected command              |
| Esc        | Dismiss popup (keeps typed text)                 |
| Any other  | Passes to input field; popup re-filters          |
