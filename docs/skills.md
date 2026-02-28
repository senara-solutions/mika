# Skills System

Skills are Mika's extensibility mechanism. Each skill is a filesystem-based bundle that packages related tools, prompt instructions, and dispatch configuration into a single directory. Skills control which tools are available to the agent on each turn, how those tools are executed, and what additional system prompt context the agent receives.

## What Are Skills?

A skill is a directory under `~/.mika/skills/<name>/` that contains:

- A **manifest** (`skill.toml`) declaring the skill's name, description, trigger keywords, handler type, and options.
- An optional **prompt snippet** (`system_prompt.md`) injected into the system prompt when the skill is active.
- An optional **tool definitions file** (`tools.json`) describing tools for exec and http handlers.

At startup, Mika scans `~/.mika/skills/` and builds a `SkillRegistry`. On each user turn, the registry matches skills against the user's message. Matched skills contribute their tools and prompt snippets to that turn's agent loop. Claude then decides which of the available tools to call.

Three built-in skills (memory, reminders, messaging) ship with Mika and are seeded into `~/.mika/skills/` on first run. You can modify these or add entirely new skills without changing any Rust code.

---

## Directory Structure

```
~/.mika/skills/
  memory/
    skill.toml              # Manifest (required)
    system_prompt.md         # Prompt snippet (optional)
  reminders/
    skill.toml
    system_prompt.md
  messaging/
    skill.toml
  custom-skill/
    skill.toml
    system_prompt.md
    tools.json              # Tool definitions for exec/http handlers (optional)
```

Each immediate subdirectory of `~/.mika/skills/` is treated as a skill. The directory name is for organization only; the skill's identity comes from the `name` field in `skill.toml`. Files that are not directories are ignored. Subdirectories missing a `skill.toml` or containing invalid TOML are logged as warnings and skipped -- they never prevent startup.

---

## Manifest Reference (skill.toml)

The manifest is a TOML file with the following structure:

### Top-level fields

| Field         | Type   | Required | Description                              |
|---------------|--------|----------|------------------------------------------|
| `name`        | String | Yes      | Unique skill name (used in prompt headers and logging). |
| `description` | String | Yes      | Human-readable description of the skill. |

### `[triggers]` section

| Field      | Type           | Required | Default | Description                                     |
|------------|----------------|----------|---------|-------------------------------------------------|
| `keywords` | Array<String>  | No       | `[]`    | Keywords for substring matching against user messages. Case-insensitive. |

If the `[triggers]` section is omitted entirely, it defaults to an empty keyword list. A skill with no keywords and `always_on = false` will never activate.

### `[handler]` section

| Field     | Type                     | Required | Description                                              |
|-----------|--------------------------|----------|----------------------------------------------------------|
| `type`    | `"builtin"` / `"exec"` / `"http"` | Yes      | Dispatch method for tool calls.                          |
| `tools`   | Array<String>            | Yes      | Tool names this handler owns (all handler types).        |
| `command` | String                   | Exec only | Path to the executable to run.                          |
| `args`    | Array<String>            | No       | Static arguments passed before the tool name (exec only). Default: `[]`. |
| `url`     | String                   | Http only | URL to POST tool calls to.                              |
| `headers` | Map<String, String>      | No       | HTTP headers added to every request (http only). Default: `{}`. |

The `type` field uses `#[serde(tag = "type")]` deserialization, so it must appear as a string value within the `[handler]` table.

### `[options]` section

| Field          | Type | Required | Default | Description                                      |
|----------------|------|----------|---------|--------------------------------------------------|
| `always_on`    | bool | No       | `false` | If true, this skill is active on every turn regardless of keywords. |
| `timeout_secs` | u64  | No       | `30`    | Per-tool execution timeout in seconds (applies to exec and http handlers). |

If the `[options]` section is omitted, both fields take their defaults.

### Minimal valid manifest

```toml
name = "minimal"
description = "A minimal skill with no tools"

[handler]
type = "builtin"
tools = []
```

---

## Handler Types

### Builtin

References tools already registered in Mika's Rust `ToolRegistry`. The `tools` array lists tool names that exist in compiled Rust code. No `tools.json` file is needed; tool definitions are pulled from the registry at runtime.

```toml
name = "memory"
description = "Manage persistent memory, core memory blocks, and stored facts"

[triggers]
keywords = ["remember", "memory", "fact", "person", "commitment", "preference", "event"]

[handler]
type = "builtin"
tools = ["update_core_memory", "store_fact", "search_memory", "update_fact"]

[options]
always_on = true
```

When Claude calls a builtin tool, it is dispatched through the standard `ToolRegistry` with full access to the `ToolContext` (database, session, home directory, etc.).

### Exec

Runs a shell command for each tool call. The command receives the tool name as an additional trailing argument and the tool input JSON via the `MIKA_TOOL_INPUT` environment variable. Stdout is returned as the tool result; a non-zero exit code is reported as an error.

```toml
name = "calendar"
description = "Calendar integration via local script"

[triggers]
keywords = ["calendar", "meeting", "schedule", "event"]

[handler]
type = "exec"
command = "/usr/local/bin/cal-tool"
args = ["--format", "json"]
tools = ["get_events", "create_event"]

[options]
timeout_secs = 15
```

Execution details:

- The command is spawned as: `<command> <args...> <tool_name>`
- For the example above, calling `get_events` runs: `/usr/local/bin/cal-tool --format json get_events`
- The tool input JSON is available in the `MIKA_TOOL_INPUT` environment variable.
- The process must complete within `timeout_secs` or it is killed and an error is returned.
- Stdout is captured as the successful tool result.
- Stderr is included in the error message on non-zero exit.

Exec handlers require a `tools.json` file in the skill directory to define the tool schemas sent to Claude.

### Http

POSTs tool calls to a URL. The request body is JSON with `tool_name` and `input` fields. A 2xx response body is returned as the tool result; non-2xx status codes are reported as errors.

```toml
name = "weather"
description = "Weather lookup via external API"

[triggers]
keywords = ["weather", "forecast", "temperature"]

[handler]
type = "http"
url = "http://localhost:8080/tools"
tools = ["get_weather"]

[handler.headers]
Authorization = "Bearer token123"
X-Custom-Header = "value"

[options]
timeout_secs = 10
```

Execution details:

- The HTTP POST body is:
  ```json
  {
    "tool_name": "get_weather",
    "input": { "city": "Tokyo" }
  }
  ```
- All entries in `[handler.headers]` are added to the request.
- The request timeout is controlled by `timeout_secs`.
- A successful (2xx) response body is returned verbatim as the tool result.
- Non-2xx responses return `"HTTP <status>: <body>"` as an error.
- Connection failures and timeouts are reported as errors.

Http handlers require a `tools.json` file in the skill directory to define the tool schemas sent to Claude.

---

## Trigger Matching

On each user turn, the `SkillRegistry` determines which skills are active:

1. **Always-on skills** are included unconditionally, regardless of message content. Set `always_on = true` in `[options]`.

2. **Keyword-matched skills** are included when at least one keyword from their `triggers.keywords` list appears as a substring of the user's message. Matching is case-insensitive: keywords are pre-lowercased at scan time, and the user message is lowercased before comparison.

3. **Unmatched skills** are excluded from the turn entirely. Their tools are not sent to Claude and their prompt snippets are not injected.

The matching algorithm is intentionally simple and cheap. Claude still makes the final decision about which tools to actually call from the matched set. The keyword system acts as a coarse filter to avoid sending irrelevant tools on every turn.

**Fallback behavior:** If no skills directory exists or the `SkillRegistry` has zero loaded skills, all builtin tools are sent to Claude on every turn (pre-skill legacy behavior). This ensures Mika works out of the box even if the skills directory is missing.

**Silent agent loop:** For background tasks (heartbeats, reminders), matching runs against a synthetic trigger string derived from the task type. For heartbeats, this is `"heartbeat check-in send message reminder"`. For reminders, it is the reminder message text.

---

## Prompt Snippets (system_prompt.md)

Each skill directory may contain a `system_prompt.md` file. When a skill is matched for a turn, the snippet is lazy-loaded from disk and injected into the system prompt in the format:

```
## <name> Skill
<contents of system_prompt.md>
```

For example, the memory skill's `system_prompt.md`:

```markdown
- Update your core memory when you learn important things about the user.
- Track people, commitments, preferences, and events using the appropriate tools.
- Use search_memory to find stored facts across all categories before asking the user to repeat information.
- Mark commitments as completed or cancelled using the update_fact tool.
- You can reset a core memory section to its default value using update_core_memory with the reset action.
```

This is injected as:

```
## memory Skill
- Update your core memory when you learn important things about the user.
- Track people, commitments, preferences, and events using the appropriate tools.
...
```

If the file is missing or empty, no snippet is injected for that skill. Snippets are loaded asynchronously (`tokio::fs::read_to_string`) on each turn, so changes to the file take effect immediately without restarting Mika.

---

## Tool Definitions (tools.json)

Exec and http handlers need a `tools.json` file to tell Claude what tools are available and what inputs they accept. This file is not needed for builtin handlers (their definitions come from the Rust `ToolRegistry`).

The file contains a JSON array of `ToolDefinition` objects:

```json
[
  {
    "name": "get_weather",
    "description": "Get the current weather for a city",
    "input_schema": {
      "type": "object",
      "properties": {
        "city": {
          "type": "string",
          "description": "City name (e.g., 'Tokyo', 'New York')"
        }
      },
      "required": ["city"]
    }
  }
]
```

### ToolDefinition fields

| Field          | Type   | Required | Description                                                |
|----------------|--------|----------|------------------------------------------------------------|
| `name`         | String | Yes      | Tool name. Must appear in the handler's `tools` array.     |
| `description`  | String | Yes      | Description shown to Claude explaining what the tool does. |
| `input_schema` | Object | Yes      | JSON Schema object describing the tool's input parameters. |

Only tool definitions whose `name` appears in the handler's `tools` list are sent to Claude. Extra definitions in `tools.json` that are not listed in the handler's `tools` array are ignored.

Tool definitions are loaded lazily on each turn (via `tokio::fs::read_to_string` and `serde_json` deserialization). Changes take effect immediately without restart.

---

## Built-in Skills Reference

These three skills are seeded into `~/.mika/skills/` on first bootstrap:

| Skill      | Always On | Keywords                                                       | Tools                                                             | Prompt Snippet |
|------------|-----------|----------------------------------------------------------------|-------------------------------------------------------------------|----------------|
| memory     | Yes       | remember, memory, fact, person, commitment, preference, event  | `update_core_memory`, `store_fact`, `search_memory`, `update_fact` | Yes            |
| reminders  | Yes       | remind, reminder, schedule, alarm, alert                       | `create_reminder`, `list_reminders`, `cancel_reminder`             | Yes            |
| messaging  | Yes       | send, message, notify                                          | `send_message`                                                     | No (empty)     |

All three use the `builtin` handler type, so their tools are dispatched through the Rust `ToolRegistry` with full access to the database and agent context. All three are `always_on = true`, meaning they are active on every turn regardless of message content.

---

## Creating a Custom Skill

This walkthrough creates an exec-based skill that converts between time zones using a shell script.

### Step 1: Create the skill directory

```bash
mkdir -p ~/.mika/skills/timezone
```

### Step 2: Write skill.toml

Create `~/.mika/skills/timezone/skill.toml`:

```toml
name = "timezone"
description = "Convert times between time zones"

[triggers]
keywords = ["timezone", "time zone", "convert time", "what time"]

[handler]
type = "exec"
command = "/home/you/.mika/skills/timezone/handler.sh"
tools = ["convert_timezone"]

[options]
timeout_secs = 10
```

### Step 3: Write the handler script

Create `~/.mika/skills/timezone/handler.sh`:

```bash
#!/bin/bash
set -euo pipefail

# Tool name is passed as the last argument
TOOL="$1"

# Tool input is in MIKA_TOOL_INPUT (JSON string)
INPUT="${MIKA_TOOL_INPUT:-{}}"

case "$TOOL" in
  convert_timezone)
    # Parse input fields using jq
    TIME=$(echo "$INPUT" | jq -r '.time')
    FROM_TZ=$(echo "$INPUT" | jq -r '.from_timezone')
    TO_TZ=$(echo "$INPUT" | jq -r '.to_timezone')

    # Convert using date command
    RESULT=$(TZ="$TO_TZ" date -d "TZ=\"$FROM_TZ\" $TIME" '+%Y-%m-%d %H:%M:%S %Z' 2>&1)

    echo "{\"converted_time\": \"$RESULT\", \"from\": \"$FROM_TZ\", \"to\": \"$TO_TZ\"}"
    ;;
  *)
    echo "Unknown tool: $TOOL" >&2
    exit 1
    ;;
esac
```

Make it executable:

```bash
chmod +x ~/.mika/skills/timezone/handler.sh
```

### Step 4: Write tools.json

Create `~/.mika/skills/timezone/tools.json`:

```json
[
  {
    "name": "convert_timezone",
    "description": "Convert a time from one timezone to another. Use IANA timezone names (e.g., 'America/New_York', 'Asia/Tokyo', 'Europe/London').",
    "input_schema": {
      "type": "object",
      "properties": {
        "time": {
          "type": "string",
          "description": "The time to convert (e.g., '2024-03-15 14:30:00' or '2:30 PM')"
        },
        "from_timezone": {
          "type": "string",
          "description": "Source IANA timezone (e.g., 'America/New_York')"
        },
        "to_timezone": {
          "type": "string",
          "description": "Target IANA timezone (e.g., 'Asia/Tokyo')"
        }
      },
      "required": ["time", "from_timezone", "to_timezone"]
    }
  }
]
```

### Step 5: Write system_prompt.md

Create `~/.mika/skills/timezone/system_prompt.md`:

```markdown
- Use convert_timezone to convert times between time zones when the user asks.
- Always use IANA timezone names (e.g., America/New_York, not EST).
- If the user gives an ambiguous timezone abbreviation, ask for clarification or use the most common interpretation.
```

### Step 6: Test it

A restart is required for Mika to discover new skill directories, because `skill.toml` manifests are scanned once at startup. After restarting Mika (or the mika-server process), send a message like:

```
What time is it in Tokyo when it's 3 PM in New York?
```

The keyword `"time"` in the message matches `"what time"` from the triggers (substring match), so the timezone skill activates. Claude receives the `convert_timezone` tool definition and the prompt snippet, and can call the tool to answer.

If the skill fails to load, check the Mika logs for warnings. Common issues:

- Invalid TOML syntax in `skill.toml`
- Handler script not executable (`chmod +x`)
- `tools.json` not valid JSON or missing required fields
- Tool names in `tools.json` not matching the `tools` array in `skill.toml`

---

## Customizing Built-in Skills

The three built-in skills (memory, reminders, messaging) are seeded into `~/.mika/skills/` only on first bootstrap. If a skill directory already exists, the bootstrap process skips it entirely. This means you can safely modify built-in skills and your changes will persist across upgrades.

### Example: Disable always-on for reminders

Edit `~/.mika/skills/reminders/skill.toml` and change:

```toml
[options]
always_on = false
```

Restart Mika for the manifest change to take effect. The reminders tools will then only be available when the user's message contains one of the trigger keywords (remind, reminder, schedule, alarm, alert).

### Example: Add keywords to a skill

Edit `~/.mika/skills/memory/skill.toml` and add keywords:

```toml
[triggers]
keywords = ["remember", "memory", "fact", "person", "commitment", "preference", "event", "note", "important"]
```

Restart Mika for the manifest change to take effect.

### Example: Change the prompt snippet

Edit `~/.mika/skills/memory/system_prompt.md` to add custom instructions:

```markdown
- Update your core memory when you learn important things about the user.
- Always confirm with the user before storing sensitive personal information.
- Prefer search_memory before asking the user to repeat something.
```

Prompt snippet changes take effect on the next message (no restart needed), because `system_prompt.md` is lazy-loaded from disk each turn.

### Disabling bundled skill re-sync

By default, Mika re-syncs bundled skill definitions from compiled-in templates on
every startup, ensuring template updates propagate to existing installations. If
you are editing handler scripts and want changes to survive restarts, set:

```toml
# In ~/.mika/config.toml
disable_bundled_skills = true
```

Or via environment variable:

```sh
export MIKA_DISABLE_BUNDLED_SKILLS=true
```

**Warning:** Do not enable this in production. It prevents security updates to
handler scripts (e.g., shell-exec, file-reader) from being applied on restart.

The `mika agents create` command always seeds skills regardless of this setting.

### Resetting to defaults

To reset a built-in skill to its shipped defaults, delete the skill directory and restart Mika. The bootstrap process will recreate it:

```bash
rm -rf ~/.mika/skills/memory
# Restart Mika -- bootstrap recreates the memory skill from templates
```

---

## Security Considerations

### Exec handlers run unsandboxed

Exec handlers spawn a child process with `tokio::process::Command`. The process runs with the same user permissions as Mika itself. There is no sandboxing, chroot, or capability restriction.

**Environment variable leakage:** The child process inherits Mika's full environment, which may include `MIKA_ANTHROPIC_API_KEY` and other secrets. The only environment variable explicitly set by Mika is `MIKA_TOOL_INPUT` (containing the tool's JSON input). If you run untrusted exec handlers, secrets in the environment are accessible to them.

**Mitigation:** Only install exec skills from sources you trust. Review handler scripts before placing them in `~/.mika/skills/`. Consider using http handlers to isolate untrusted tools behind a network boundary.

### Http handlers make network requests

Http handlers POST to the configured URL using `reqwest`. The request includes the tool name and input, plus any headers defined in the manifest. This means:

- Tool input data (which may contain user messages or personal information) is sent over the network to the configured endpoint.
- Custom headers (e.g., `Authorization`) are stored in plaintext in `skill.toml`.
- There is no certificate pinning or URL allowlisting beyond what you configure.

**Mitigation:** Use HTTPS URLs for http handlers. Keep `skill.toml` files readable only by the Mika user (bootstrap sets `~/.mika/` to mode 0700 on Unix). Avoid storing long-lived secrets directly in `skill.toml`; consider having the http endpoint handle its own authentication.

### Trust boundary

The skills directory (`~/.mika/skills/`) is the trust boundary. Anyone who can write files to this directory can:

- Add new tools that Mika will offer to Claude.
- Inject arbitrary text into the system prompt via `system_prompt.md`.
- Execute arbitrary commands (exec handler) or make arbitrary HTTP requests (http handler) with Mika's privileges.

Protect `~/.mika/skills/` with appropriate filesystem permissions. On Unix, bootstrap sets the directory to mode 0700 (owner-only access).

### Timeout enforcement

Both exec and http handlers enforce the `timeout_secs` value from the manifest (default: 30 seconds). Exec processes that exceed the timeout are killed. HTTP requests that exceed the timeout are cancelled. This prevents a misbehaving handler from blocking the agent loop indefinitely.

### Invalid skills never break startup

The skill scanner (`scan_skills_dir`) catches all errors -- missing files, invalid TOML, bad JSON -- and logs them as warnings. A broken skill is simply skipped. This ensures that a single bad skill file cannot prevent Mika from starting.
