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

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | `/h`, `/?`               |
| Arguments   | None                     |

**Example output:**

```
Available commands:
  /help (h, ?) — List available commands
  /clear [--all] — Clear chat display (--all for DB)
  /exit (quit, q) — Quit mika
  /compact — Compact conversation history
  /memory (mem) [search <query>] — Show core memory blocks
  /reminders (remind) — List active reminders
  /status (stat) — Show system health info
  /soul — Display current soul.md
  /config (cfg) — Show current config
  /model — Show active model
  /export — Export conversation to markdown
  /skills — List loaded skills
  /skill <name> — Show skill details
```

---

### /clear

Clear all messages from the chat display and reset the scroll position.

| Detail      | Value                       |
|-------------|-----------------------------|
| Aliases     | None                        |
| Arguments   | `[--all]` (optional)        |

**Behavior:** Empties the in-memory message list and resets `scroll_offset` to 0.
The display is cleared immediately; the underlying database conversation is not
affected unless `--all` is passed.

**Example output:**

```
Chat display cleared.
```

---

### /exit

Quit the Mika TUI. Sets `should_quit = true` and produces no output.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | `/quit`, `/q`            |
| Arguments   | None                     |

---

### /compact

Manually trigger conversation compaction. The agent's conversation history is
summarized and trimmed to reduce token usage.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | None                     |

**Requirements:**

- The agent must be idle (not processing a turn).
- The conversation must have more than 50 messages.

**Possible outputs:**

```
Compacted conversation (73 messages).
```

```
Nothing to compact (12/50 messages).
```

```
Cannot compact while agent is busy.
```

---

### /memory

Show core memory blocks or search across all structured memory layers.

| Detail      | Value                             |
|-------------|-----------------------------------|
| Aliases     | `/mem`                            |
| Arguments   | `[search <query>]` (optional)     |

**Without arguments** -- displays all core memory entries with their keys, values,
and token counts, plus the total token usage against the 2000-token limit:

```
Core Memory:
  [user_name] Sami (12 tokens)
  [user_role] CTO at Senara Solutions (18 tokens)
  [personality] Direct, technical, prefers concise answers (22 tokens)

Total: 52/2000 tokens
```

**With `search <query>`** -- searches across all Layer 2 memory tables (People,
Commitments, Preferences, Events) concurrently and displays results grouped by
category:

```
/memory search meeting
```

```
Commitments:
  [active] Review Q4 roadmap with team (due: 2026-03-01)
Events:
  Board meeting with investors (2026-03-15)
```

If no results match:

```
No results for 'meeting'.
```

Missing search query:

```
Usage: /memory search <query>
```

---

### /reminders

List all active reminders, split into pending (past-due) and upcoming (future).

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | `/remind`                |
| Arguments   | None                     |

**Example output:**

```
Pending reminders:
  #3: Follow up with Alice on contract (due: 2026-02-24T10:00:00Z)
Upcoming reminders:
  #5: Weekly team standup (fires: 2026-02-26T09:00:00Z)
  #7: Send invoice to client (fires: 2026-02-28T14:00:00Z)
```

If there are no reminders:

```
No active reminders.
```

---

### /status

Show system health information including message count, database size, core memory
usage, schema version, active model, and session ID.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | `/stat`                  |
| Arguments   | None                     |

All four database queries (message count, DB size, core memory tokens, schema
version) run concurrently via `tokio::join!`.

**Example output:**

```
Status:
  Messages: 142
  DB size: 384 KB
  Core memory: 52/2000 tokens
  Schema: v6
  Model: claude-sonnet-4-6
  Session: a1b2c3d4
```

---

### /soul

Display the contents of the user's `soul.md` file, which provides personality and
behavioral guidance injected into the agent's system prompt.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | None                     |

**Possible outputs:**

If `~/.mika/soul.md` exists and has content, its full text is displayed.

If the file is empty:

```
soul.md is empty.
```

If the file does not exist:

```
No soul.md found. Create one at ~/.mika/soul.md
```

---

### /config

Show the current configuration summary. Does not dump file contents (which may
contain secrets) -- only shows key settings and the config file path.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | `/cfg`                   |
| Arguments   | None                     |

**Example output:**

```
Configuration:
  Model: claude-sonnet-4-6
  Home: /home/sami/.mika
  Session: a1b2c3d4
  Config file: /home/sami/.mika/config/local.toml
```

If no local config file exists:

```
Configuration:
  Model: claude-sonnet-4-6
  Home: /home/sami/.mika
  Session: a1b2c3d4
  Config file: (using defaults)
```

---

### /model

Show the currently active Claude model.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | None                     |

**Example output:**

```
Current model: claude-sonnet-4-6
```

---

### /export

Export the current conversation to a Markdown file in `~/.mika/exports/`.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | None                     |

**Behavior:**

- Creates the `exports/` directory if it does not exist.
- Generates a filename using the session ID prefix and UTC timestamp:
  `session-a1b2c3d4-2026-02-25-143022.md`
- Writes User, Assistant, and System messages. Command output (from slash commands)
  is intentionally excluded as ephemeral.
- Uses `create_new` to prevent overwriting existing files (symlink-attack safe).

**Example output:**

```
Exported to /home/sami/.mika/exports/session-a1b2c3d4-2026-02-25-143022.md
```

If the chat is empty:

```
Nothing to export.
```

---

### /skills

List all loaded skills from the filesystem-based skill registry.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | None                     |

Each skill shows its name, handler type (`builtin`, `exec`, or `http`), description,
and whether it is marked as always-on.

**Example output:**

```
Loaded skills:
  web_search (builtin) — Search the web for current information
  calendar (exec) — Manage calendar events and scheduling [always on]
  code_review (http) — Review code and suggest improvements
```

If no skills are loaded:

```
No skills loaded.
```

---

### /skill

Show detailed information about a specific loaded skill.

| Detail      | Value                    |
|-------------|--------------------------|
| Aliases     | None                     |
| Arguments   | `<name>` (required)      |

Skill lookup is case-insensitive.

**Example output:**

```
/skill web_search
```

```
Skill: web_search
  Description: Search the web for current information
  Handler: builtin (tools: web_search, browse_url)
  Always on: false
  Keywords: search, look up, find online
  Path: /home/sami/.mika/skills/web_search
```

If the name is omitted:

```
Usage: /skill <name>
```

If no matching skill is found:

```
No skill found with name 'foo'. Use /skills to list all loaded skills.
```

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
