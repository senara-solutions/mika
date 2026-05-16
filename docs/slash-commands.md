---
title: Slash Commands
description: TUI slash commands for navigation, memory, and agent control
---

# Slash Commands Reference

## Overview

Slash commands are client-side actions executed directly in the Mika TUI. They are
**never sent to the agent** -- typing `/status` runs local code that queries the
database and renders output in-place, while typing a regular message (no `/` prefix)
sends it to the Claude-backed agent loop as usual.

All 24 commands are defined in a single `COMMANDS` array
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

1. Type `/` -- a popup appears listing all 23 commands.
2. Continue typing to narrow the list. For example, `/me` narrows to `/memory`.
   Matching works on both command names and aliases (e.g., typing `/q` matches
   `/exit` via its `q` alias).

### Argument Completion

Commands that accept arguments support Tab-triggered argument completion. After
accepting a command name, press Tab to see available completions:

| Command | Completes | Source |
|---------|-----------|--------|
| `/model` | Model aliases + cached provider models | Static + `cache/models/{provider}.json` |
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

Clear the chat display and start a new session.

**Aliases:** None | **Arguments:** None

Ends the current session, creates a new one with a fresh UUID, and notifies the agent worker. Drains any stale responses from the agent channel to prevent ghost messages. Clears all display messages, resets scroll position, context token tracking, cross-channel polling watermarks, and all transient state: `pending_response`, `reveal_index`, `status` (back to Idle), `pending_images`, `pending_command`, `has_new_message`, `selection_state`, `pending_task_count`. User preferences (`thinking_level`, model, provider) are preserved. `active_background_task_count` is intentionally NOT reset — background callback tasks are agent-scoped and persist across sessions. The agent starts fresh with no conversation history. Previous sessions remain in the database for audit and history purposes.

---

### /exit

Quit the Mika TUI. Sets `should_quit = true` and produces no output. Works regardless of agent status (Idle, Thinking, or Responding) — critical for `tmux send-keys` scripts that need to stop autonomous agents mid-turn.

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
Status: Messages: 142 | DB size: 384 KB | Core memory: 52/2000 tokens | Schema: v9 | Model: claude-sonnet-4-6 | Session: a1b2c3d4
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
Configuration: Model: claude-sonnet-4-6 | Home: /home/sami/.mika | Session: a1b2c3d4 | Config file: /home/sami/.mika/config.toml
```

---

### /model

Show or switch the currently active LLM model. Without arguments, displays the
current model and lists available models for the active provider. With an argument,
switches to that model immediately.

**Aliases:** None | **Arguments:** `[model-name|alias|provider/model]` (optional)

**Show current model and available models:**
```
/model
→ Current model: claude-sonnet-4-6

Available models for anthropic:
  claude-sonnet-4-6 (current)
  claude-opus-4-6
  claude-haiku-4-5

Aliases:
  sonnet — Claude Sonnet 4.6
  opus — Claude Opus 4.6
  ...
```

For Anthropic and Google, models are hardcoded. For all other providers (OpenAI,
DeepSeek, Groq, etc.), models are fetched from the provider's `/models` API and
cached per-agent at `cache/models/{provider}.json` with a 24-hour TTL. If the API
is unreachable, stale cache or aliases are used as fallback.

**Switch model by alias:**
```
/model opus
→ Switched to Claude Opus 4.6 (claude-opus-4-6).
```

**Switch model by name (any model the provider supports):**
```
/model deepseek-reasoner
→ Switched to deepseek/deepseek-reasoner.
```

**Cross-provider switch (alias or provider/model format):**
```
/model deepseek
→ Switched to DeepSeek Chat (deepseek/deepseek-chat). (switched provider to deepseek)
```

When a model alias or `provider/model` format targets a different provider, the
active provider is also switched and both `llm_provider` and `{provider}_model`
are persisted to `config.toml`.

Recognized aliases: `sonnet`, `opus`, `haiku`, `gpt4o`, `deepseek`, `gemini`.

---

### /provider

Show or switch the active LLM provider. Supports per-field configuration for model,
API key, and base URL.

**Aliases:** None | **Arguments:** `[anthropic|openai|groq|ollama|...]` (optional)

**Show current provider:**
```
/provider
→ Current provider: anthropic

Available providers:
  anthropic — claude-sonnet-4-6 (current)
  openai — gpt-4o
  groq — llama-3.3-70b-versatile
  ...
```

**Switch provider:**
```
/provider deepseek
→ Switched to deepseek (model: deepseek-reasoner).
Note: anthropic_model is still set (kept for switching back)
```

The switch reads the user's configured `{provider}_model` from `config.toml` if
set, falling back to the provider's default model otherwise. When no model was
previously configured, the default is persisted to `config.toml` for visibility.

Switching warns about stale model fields from the previous provider (kept for
switching back) and if `llm_max_tokens` exceeds the new provider's limit.

Validates the provider configuration before switching — if the required API key is
missing, the switch is blocked with a clear error message:
```
/provider openai
→ Cannot switch to openai: Missing API key for provider openai
```

**Set provider fields:**
```
/provider set model gpt-4-turbo     → Set openai_model = "gpt-4-turbo"
/provider set api_key sk-...        → Set MIKA_OPENAI_API_KEY in .env (restart required)
/provider set base_url https://...  → Set openai_base_url = "https://..."
```

Provider switches are persisted to `config.toml`. If persistence fails, the switch
still takes effect for the current session with a warning.

---

### /export

Export the current conversation to a Markdown file in `~/.mika/exports/`.

**Aliases:** None | **Arguments:** None

Creates the `exports/` directory if needed. Generates a filename using session ID prefix and UTC timestamp (e.g., `session-a1b2c3d4-2026-02-25-143022.md`). Writes User, Assistant, and System messages; command output is excluded as ephemeral. Uses `create_new` to prevent overwrites (symlink-attack safe). Output: `Exported to /home/sami/.mika/exports/session-a1b2c3d4-2026-02-25-143022.md` or `Nothing to export.`

---

### /skills

List all loaded skills from the filesystem-based skill registry.

**Aliases:** None | **Arguments:** None

Skills are grouped into "ALWAYS ON" and "ON DEMAND" sections. Each skill shows its name, tool count, description, and optional badges (`[disabled]`, `[override]`, `[variants: N]`). Within each group, enabled skills are listed before disabled ones. The header shows the total count and, if any skills failed to load, the skipped count.

If skills were skipped during scanning (broken symlinks, invalid manifests, oversized prompts, etc.), a "SKIPPED" section appears at the bottom showing each skipped skill's name and reason. At startup, skipped skills are also shown as a system message warning (up to 5 inline, with a note to run `mika skills validate` for details).

```
Loaded skills (12, 2 skipped):

  ALWAYS ON
  ● memory       —         Core memory management
  ● self-knowledge  —      Agent self-awareness

  ON DEMAND
  ● web-search   1 tool    Search the web

  SKIPPED
  ✗ broken-skill           broken symlink → /old/path
  ✗ bad-manifest           invalid TOML: expected `=` at line 3
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

### /tasks

List scheduled tasks from the unified task engine. Shows task ID (8-char prefix),
label, action type, status, and next fire time.

**Aliases:** None | **Arguments:** None

```
Scheduled tasks:
  a1b2c3d4  heartbeat          send_message    recurring_active  next: 2026-03-06T01:00:00Z
  e5f6a7b8  follow-up-alice    send_message    pending           next: 2026-03-07T09:00:00Z
```

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

### /inbox

Toggle between inbox mode (hide internal agent-to-agent messages) and audit mode (show all messages). Inbox mode is the default.

**Aliases:** None | **Arguments:** None

When toggled, message history is reloaded from the database with the new filter setting. In inbox mode, a `[N hidden]` footer badge shows the count of suppressed internal messages.

**Example:**
```
/inbox
```

Output: `Switched to audit mode (all messages visible).` or `Switched to inbox mode (internal messages hidden).`

---

### /undo

Undo the last exchange (user message + assistant response) and reverse any memory changes made during that exchange. Equivalent to `/rewind 1`.

**Aliases:** None | **Arguments:** None

Uses the conversation rewind engine: previews the last exchange, shows what will be deleted and which memory mutations will be reversed, then executes the rewind. After execution, a context marker message is injected into the session so the agent knows messages were removed and does not confabulate about the gap.

```
/undo
→ Rewind complete: 2 messages removed, 1 reversal applied.
```

Errors: `Cannot rewind while agent is busy.` or `Nothing to undo.`

---

### /rewind

Rewind multiple exchanges or rewind to a specific message ID. Previews changes before executing.

**Aliases:** None | **Arguments:** `[<count> | to <message_id>]` (optional)

**Without arguments** -- rewinds 1 exchange (same as `/undo`).

**With a count:**
```
/rewind 3
→ Rewind complete: 6 messages removed, 2 reversals applied.
```

**With a target message ID:**
```
/rewind to 42
→ Rewind complete: 8 messages removed, 0 reversals applied.
```

The rewind engine automatically reverses memory mutations (core memory edits, fact stores/updates) that occurred in the rewound messages by consulting the audit log. Tasks created during the rewound period are cancelled. A context marker is injected after the rewind to inform the agent of the gap.

Cross-session rewinds are supported -- if the target messages are in a different session than the current one, the marker notes the originating session.

Errors: `Cannot rewind while agent is busy.` or `No messages to rewind.`

---

### /restart

Recover from an agent-worker crash. The TUI runs the agent loop in a supervised
`tokio::spawn` task; if that worker panics or exits prematurely, a banner appears
("Agent worker crashed: <reason>. Use /restart to recover."). `/restart` tears the
dead worker down and spawns a fresh one for the same agent (mika#1149).

**Aliases:** None | **Arguments:** None

**On a healthy worker** -- refuses:
```
/restart only applies after the worker has crashed. Use /clear to start a new session on a healthy worker.
```

**After a crash** -- arms the restart and the chat loop spawns a replacement on
its next iteration:
```
Restarting agent worker… (the lost prompt is not replayed — please re-send it after the worker comes back up.)
```

The lost in-flight prompt is **not** replayed; the operator must re-type. Background
callback tasks survive the restart and continue delivering to the new worker.
The `agent_worker_silenced` structured `error!` event is emitted to the per-agent
log on every crash for post-incident triage.

---

## Dashboard Controls

The TUI footer bar shows a colored dot indicating dashboard status (green when
enabled, red when disabled) with clickable `[start]`/`[stop]` and `[open]`
buttons. Status is polled every ~5 seconds by querying the mika-server
`GET /api/v1/dashboard/status` endpoint.

Requires `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN` to be set. Server URL
defaults to `http://localhost:8080`, overridable via `MIKA_SERVER_URL`.

The CLI commands (`mika dashboard start/stop/status/open`) remain available for
non-interactive use.

---

## Team Mode

When the TUI is launched with `mika --team <name>`, slash commands are restricted
to a safe subset. Agent-specific commands are disabled:

| Available | Disabled |
|-----------|----------|
| `/help`, `/clear`, `/exit`, `/quit` | `/model`, `/think`, `/agent`, `/switch` |
| `/export`, `/teams`, `/agents` | `/memory`, `/reminders`, `/compact`, `/soul` |
| `/status`, `/team`, `/verbose`, `/inbox` | `/config`, `/skills`, `/skill`, `/attach`, `/tasks` |
|  | `/undo`, `/rewind` |

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
| Alt+Enter   | Insert newline in input (multi-line editing)            |
| Shift+Enter | Insert newline (works on terminals that report Shift modifier with Enter) |
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
| Alt+Enter / Shift+Enter | Dismiss popup and insert newline (switches to multi-line editing) |
| Esc        | Dismiss popup (keeps typed text)                 |
| Any other  | Passes to input field; popup re-filters          |
