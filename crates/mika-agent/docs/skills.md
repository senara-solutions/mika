---
title: Skills
description: Skills extensibility mechanism, skill.toml format, and handler types
---

# Skills System

Skills are Mika's extensibility mechanism. Each skill is a filesystem-based bundle that packages related tools, prompt instructions, and dispatch configuration into a single directory. Skills control which tools are available to the agent on each turn, how those tools are executed, and what additional system prompt context the agent receives.

## What Are Skills?

A skill is a directory under `~/.mika/skills/<name>/` that contains:

- A **manifest** (`skill.toml`) declaring the skill's name, description, trigger keywords, handler type, and options.
- An optional **prompt snippet** (`system_prompt.md`) injected into the system prompt when the skill is active.
- An optional **tool definitions file** (`tools.json`) describing tools for exec and http handlers.

At startup, Mika scans `~/.mika/skills/` and builds a `SkillRegistry`. On each user turn, the registry matches skills against the user's message. Matched skills contribute their tools and prompt snippets to that turn's agent loop. Claude then decides which of the available tools to call.

Several built-in skills ship with Mika and are seeded into `~/.mika/skills/` on first run. You can modify these or add entirely new skills without changing any Rust code.

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

The manifest is a TOML file with two sections: `[skill]` (required) and `[triggers]` (optional).

### `[skill]` section

| Field          | Type   | Required | Default | Description                              |
|----------------|--------|----------|---------|------------------------------------------|
| `name`         | String | Yes      | —       | Unique skill name (used in prompt headers and logging). |
| `description`  | String | Yes      | —       | Human-readable description of the skill. |
| `version`      | String | No       | `""`    | Skill version. |
| `always_on`    | bool   | No       | `false` | If true, this skill is active on every turn regardless of keywords. For built-in skills, user overrides are stored in the `skill_overrides` DB table (not in `skill.toml`). |
| `timeout_secs` | u64    | No       | `30`    | Per-tool execution timeout in seconds. |
| `dependencies` | Array\<String\> | No | `[]` | Other skill names that should be loaded when this skill is active. One level only — no transitive resolution. |
| `max_prompt_size` | u64 | No | `None` | Override the default 16KB size limit for `system_prompt.md`. Clamped to an 80KB ceiling to prevent abuse. |

### `[triggers]` section

| Field      | Type           | Required | Default | Description                                     |
|------------|----------------|----------|---------|-------------------------------------------------|
| `keywords` | Array<String>  | No       | `[]`    | Keywords for substring matching against user messages. Case-insensitive. The skill name must NOT appear in keywords — skills are already matched by name (#510). |

If the `[triggers]` section is omitted entirely, it defaults to an empty keyword list. A skill with no keywords and `always_on = false` will never activate.

### Per-skill LLM override (DB-only)

Per-skill LLM provider and model overrides are managed exclusively via the `skill_overrides` DB table. The `[llm]` section in `skill.toml` is no longer supported (#504) — `validate_skill()` rejects it.

Use the CLI to configure per-skill LLM overrides:

```bash
mika skills llm <name> set <provider>/<model>   # Set override
mika skills llm <name> reset                     # Remove override
mika skills llm <name> show                      # Show effective LLM
```

**Resolution order** (highest to lowest priority):

1. DB override (`skill_overrides.llm_provider` / `llm_model`) — set via CLI above
2. Per-provider agent config (e.g., `anthropic.model` from config.toml)
3. Agent global `llm_provider` + provider default model

**Conflict resolution:** If multiple matched skills declare different LLM overrides, the agent falls back to the default provider with a warning log. Skills with identical overrides are deduplicated.

The resolved provider/model feeds into the existing variant resolution system:
- `resolve_prompt(provider, model)` selects the best prompt variant
- `effective_timeout(provider, model)` selects the correct timeout

### Minimal valid manifest

```toml
[skill]
name = "minimal"
description = "A minimal skill with no tools"
```

> **Note:** Handler configuration (exec, http, builtin) lives in `tools.json` per-tool, not in `skill.toml`. See [Tool Definitions](#tool-definitions-toolsjson) below.

---

## Handler Types

Each tool in `tools.json` has a `handler` object that controls how that tool is dispatched. There are three handler types: `builtin`, `exec`, and `http`.

### Builtin

References functions compiled into Mika's Rust binary. The `function` field names the builtin to call. No handler scripts are needed.

**skill.toml:**
```toml
[skill]
name = "web-search"
description = "Search the web for current information"
version = "0.1.0"
always_on = true
timeout_secs = 30

[triggers]
keywords = ["search", "look up", "find online", "google", "latest"]
```

**tools.json:**
```json
[
  {
    "name": "web_search",
    "description": "Search the web for current information on a topic.",
    "input_schema": {
      "type": "object",
      "properties": {
        "query": {"type": "string", "description": "Search query"}
      },
      "required": ["query"]
    },
    "handler": {"type": "builtin", "function": "web_search"}
  }
]
```

When Claude calls a builtin tool, it is dispatched through the standard `ToolRegistry` with full access to the `ToolContext` (database, session, home directory, etc.).

### Exec

Runs a shell command for each tool call. The command path is resolved relative to the skill directory. The tool input JSON is piped to the process via **stdin**. Stdout is returned as the tool result; a non-zero exit code is reported as an error.

**skill.toml:**
```toml
[skill]
name = "github"
description = "Interact with GitHub using the gh CLI"
version = "0.1.0"
always_on = false
timeout_secs = 30

[triggers]
keywords = ["github", "pull request", "open pr", "create issue"]
```

**tools.json:**
```json
[
  {
    "name": "run_gh",
    "description": "Execute a GitHub CLI (gh) command.",
    "input_schema": {
      "type": "object",
      "properties": {
        "command": {"type": "string", "description": "The gh subcommand and arguments"}
      },
      "required": ["command"]
    },
    "handler": {"type": "exec", "command": "handlers/run.sh"}
  }
]
```

Execution details:

- The command is resolved relative to the skill directory (e.g., `handlers/run.sh` → `~/.mika/skills/github/handlers/run.sh`).
- The tool input JSON is piped to the process via **stdin** (read with `cat`, then parse with `jq`).
- The process must complete within `timeout_secs` or it is killed and an error is returned.
- Stdout is captured as the successful tool result.
- Stderr is included in the error message on non-zero exit.
- The command is **not** passed the tool name as an argument — all dispatch context is in the JSON on stdin.
- All `MIKA_*` environment variables are scrubbed from the child process. The agent's `MIKA_GITHUB_TOKEN` is then re-injected as `GH_TOKEN` so `gh` CLI calls in the handler run as the agent's GitHub identity. Note that `git push` / `git clone` over HTTPS does **not** receive credential helper injection — those operations fall back to host credentials or fail. Skill authors who need git over HTTPS should use SSH remotes instead, or open an issue to request `GIT_ASKPASS` injection.

Exec handlers require a `tools.json` file in the skill directory to define the tool schemas sent to Claude.

#### Long-Running Exec Handlers

For tasks that take longer than `timeout_secs` (e.g., code analysis, CI pipelines), set `long_running: true` in the tool's handler:

```json
{
  "name": "analyze_codebase",
  "description": "Run deep code analysis",
  "input_schema": {"type": "object", "properties": {"repo": {"type": "string"}}, "required": ["repo"]},
  "handler": {
    "type": "exec",
    "command": "handlers/analyze.sh",
    "long_running": true,
    "estimated_duration_secs": 300
  }
}
```

When `long_running` is true:
- Stdout is redirected to `/dev/null` (output is not captured).
- A callback task is created and the tool returns immediately with "task created".
- `__mika_task_id` and `__mika_agent` are injected into the input JSON on stdin.
- The script must deliver results via `mika ask --task-id <uuid> --task-complete "result text"` when complete. Intermediate calls (e.g., permission requests) can pass `--task-id` without `--task-complete` for observability correlation only.
- `estimated_duration_secs` is used to compute a timeout: `estimated * 3`, clamped to 600..7,776,000 seconds.

#### Returning Images from Exec Handlers

Exec handlers can return images alongside text using the `__mika_v1` envelope protocol. Instead of printing plain text to stdout, the script outputs a JSON object with a sentinel key:

```json
{"__mika_v1": {"text": "Screenshot saved.", "images": ["/tmp/screenshot.png"]}}
```

**Envelope schema:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `__mika_v1.text` | String | Yes | The text portion of the tool result |
| `__mika_v1.images` | Array\<String\> | Yes | Absolute file paths to image files (can be empty) |

**Detection:** The executor checks if stdout starts with `{"__mika_v1"` before attempting JSON parse. Non-matching output is treated as plain text (backward compatible).

**Image validation:** Each image file is validated before inclusion:
- Path is canonicalized (symlinks resolved)
- Must be a regular file (not a device, socket, etc.)
- Maximum 5MB raw file size (checked via metadata before reading)
- Magic-byte validation: only JPEG (`FF D8 FF`), PNG (`89 50 4E 47`), GIF (`47 49 46 38`), and WebP (`52 49 46 46...57 45 42 50`) are accepted
- Maximum 5 images per tool result

**Error handling:** If an image file is missing or invalid, an error note is appended to the text portion but the tool call does not fail. The text result is still returned.

**Example: image-returning handler**

```bash
#!/bin/sh
# Take a screenshot and return it via envelope
SCREENSHOT="/tmp/screenshot-$(date +%s).png"
grim "$SCREENSHOT"
printf '{"__mika_v1":{"text":"Screenshot captured.","images":["%s"]}}' "$SCREENSHOT"
```

For safe JSON construction with special characters in paths, use `jq`:

```bash
jq -n --arg path "$SCREENSHOT" '{"__mika_v1":{"text":"Screenshot captured.","images":[$path]}}'
```

### Http

POSTs tool calls to a URL. The request body is JSON with `tool_name` and `input` fields. A 2xx response body is returned as the tool result; non-2xx status codes are reported as errors.

**skill.toml:**
```toml
[skill]
name = "weather"
description = "Weather lookup via external API"
timeout_secs = 10

[triggers]
keywords = ["weather", "forecast", "temperature"]
```

**tools.json:**
```json
[
  {
    "name": "get_weather",
    "description": "Get the current weather for a city",
    "input_schema": {
      "type": "object",
      "properties": {
        "city": {"type": "string", "description": "City name (e.g., 'Tokyo')"}
      },
      "required": ["city"]
    },
    "handler": {
      "type": "http",
      "url": "http://localhost:8080/tools",
      "method": "POST"
    }
  }
]
```

Execution details:

- The HTTP POST body is:
  ```json
  {
    "tool_name": "get_weather",
    "input": { "city": "Tokyo" }
  }
  ```
- The request timeout is controlled by `timeout_secs` from `skill.toml`.
- A successful (2xx) response body is returned verbatim as the tool result.
- Non-2xx responses return `"HTTP <status>: <body>"` as an error.
- Connection failures and timeouts are reported as errors.
- The `method` field defaults to `"POST"` if omitted.

Http handlers require a `tools.json` file in the skill directory to define the tool schemas sent to Claude.

---

## Trigger Matching

On each user turn, the `SkillRegistry` determines which skills are active:

1. **Always-on skills** are included unconditionally, regardless of message content. Set `always_on = true` in `[skill]`.

2. **Keyword-matched skills** are included when at least one keyword from their `triggers.keywords` list appears as a substring of the user's message. Matching is case-insensitive: keywords are pre-lowercased at scan time, and the user message is lowercased before comparison.

3. **Dependency resolution:** After direct matching, matched skills that declare `dependencies = ["foo"]` pull in the named skill "foo" (if it exists and is enabled). Resolution is one level only — dependencies of dependencies are not resolved. This ensures that an `always_on` skill can reference tools from another skill without requiring keyword overlap.

4. **Unmatched skills** are excluded from the turn entirely. Their tools are not sent to Claude and their prompt snippets are not injected.

5. **Review-target exclusion (#513):** When `skill-review` is keyword-matched (i.e., the user is reviewing a skill), any other keyword-matched skill whose name appears in the user message is excluded from the matched set before prompt injection. This prevents the reviewed skill's prompt from contaminating the review context. `AlwaysOn` and `Dependency` skills are never excluded by this mechanism. Applied in both conversation and team mode via `review_filter::apply_review_filter()`.

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

### Size limit

Prompt snippets are subject to a size limit to prevent excessive token usage. The default limit is **16KB**. For `always_on` skills, exceeding the limit causes the skill to be **skipped entirely** at startup (logged at `error!` level) — an `always_on` skill without its prompt is functionally broken. For non-`always_on` skills, the prompt is discarded (logged at `error!` level) but the skill still loads. To allow a larger prompt, set `max_prompt_size` in `skill.toml`:

```toml
[skill]
name = "large-prompt-skill"
description = "Skill with a large system prompt"
max_prompt_size = 32768  # 32KB
```

The `max_prompt_size` value is clamped to a hard ceiling of **80KB** regardless of what is specified. Use `mika skills validate` to check whether a skill's prompt exceeds its effective limit.

### Per-Provider and Per-Model Variant Directories

Skills can ship model-tuned prompt variants alongside the root prompt. The variant hierarchy supports two levels of nesting: **provider** directories (matching `ProviderKind` values) and **model** subdirectories within each provider:

```
~/.mika/skills/web-search/
├── skill.toml              # Root manifest (required)
├── system_prompt.md        # Root prompt (fallback for all models)
├── tools.json              # Tool definitions (NOT overridable per-variant)
├── handlers/               # Handler scripts (NOT overridable per-variant)
├── anthropic/              # Provider directory (overrides + model subdirs)
│   ├── skill.toml          # Sparse overrides: timeout_secs, max_prompt_size (optional)
│   └── claude-sonnet-4-6/  # Model variant directory
│       ├── system_prompt.md  # Sonnet 4.6-specific prompt
│       └── skill.toml        # Sparse overrides (optional)
├── openai/
│   ├── skill.toml          # OpenAI-wide timeout override
│   └── gpt-4o/
│       └── system_prompt.md  # GPT-4o-specific prompt
└── openrouter/
    └── anthropic--claude-sonnet-4/   # Slash in model name → '--' separator
        └── system_prompt.md
```

**Important:** Provider-level `system_prompt.md` files are **not supported** and will be ignored. Models from the same provider (e.g., gpt-4o vs gpt-5) have different prompt requirements, so prompts must be specified at the model level. Provider directories hold only `skill.toml` overrides and model subdirectories.

Valid provider directory names: `anthropic`, `openai`, `openrouter`, `groq`, `ollama`, `mistral`, `google`, `deepseek`, `minimax`, `kimi`, `qwen`. Subdirectories with non-matching names (e.g., `handlers/`, `.git/`) are silently ignored. Within provider directories, any subdirectory (except dotdirs like `.git/`) is treated as a model variant.

**Model directory naming:** Model names map directly to directory names. For models containing slashes (common with OpenRouter, e.g., `anthropic/claude-sonnet-4`), replace `/` with `--` in the directory name: `anthropic--claude-sonnet-4`. The `sanitize_model_dir_name()` function applies this same transformation at lookup time, ensuring consistent matching.

#### Resolution Order

When injecting a skill's prompt, the agent loop resolves the best prompt using a two-level fallback:

1. `{skill}/{provider}/{sanitized_model}/system_prompt.md` → model-specific (most specific)
2. `{skill}/system_prompt.md` → root (fallback)

The first match wins. The active provider comes from `llm.provider_name()` and the active model from `llm.model_name()`. For numeric config overrides (`timeout_secs`, `max_prompt_size`), a three-level fallback applies: model override > provider override > root manifest.

#### Sparse Manifest Overrides

A `skill.toml` inside a provider or model directory is **sparse** — it contains only the fields that differ from the root manifest. Fields absent from the variant retain their root values.

**Overridable fields:** `timeout_secs`, `max_prompt_size`.

**Not overridable per-variant:** `name`, `description`, `version`, `dependencies`, `[triggers]`, `tools.json`, handler scripts. These are identity/structural fields and must remain consistent across all variants.

Example `openai/skill.toml` (provider-level):
```toml
[skill]
timeout_secs = 60
```

Example `openai/gpt-4o/skill.toml` (model-level):
```toml
[skill]
timeout_secs = 120
```

This gives GPT-4o a 120-second timeout, other OpenAI models get 60 seconds, and all non-OpenAI providers use the root manifest's value.

#### Validation

`mika skills validate` checks provider and model variant directories:

- Warns if a provider subdirectory name looks like a misspelling of a known provider
- Warns if a provider directory contains `system_prompt.md` (not supported — use model-level prompts instead)
- Validates `system_prompt.md` size against the effective limit (at model level)
- Validates `skill.toml` parseability (at both provider and model levels)
- Warns if a variant `skill.toml` contains identity fields (`name`, `description`) or `[triggers]` — these are silently ignored at runtime
- Warns if a variant directory contains `tools.json` (not supported at any variant level)
- Warns if a provider directory is empty (no `skill.toml` or model subdirectories)
- Warns if a model variant directory is empty (no `system_prompt.md` or `skill.toml`)
- Warns about unexpected subdirectories deeper than the model level (only two levels of nesting supported: provider/model)

#### Notes

- All variants (provider and model) are loaded eagerly at startup — no filesystem access at request time
- `SkillEntry` stores model variants in `model_prompts` and `model_overrides` maps, keyed by `"{provider}/{sanitized_model}"`
- `resolve_prompt(provider, model)` implements two-level fallback (model → root); `effective_timeout(provider, model)` implements three-level fallback (model → provider → root)
- Runtime provider/model switching (`/provider`, `/model` commands) selects the correct variant without re-scanning
- Skills without variant directories work exactly as before (zero overhead)
- `mika skills install` copies or symlinks the entire skill directory including all variant subdirectories
- `mika skills list` shows `[variants: N]` badge (N = total distinct provider + model entries)
- `mika skills info <name>` shows providers with model variants nested underneath (e.g., `anthropic (1 models)`)
- TUI `/skill <name>` shows model variants nested under providers with tree-style indentation

---

## Tool Definitions (tools.json)

Skills that provide tools need a `tools.json` file to define the tool schemas sent to Claude **and** the handler dispatch config for each tool. This file is required for exec and http tools; builtin-handler skills also use it to declare which builtin function each tool maps to.

The file contains a JSON array of `SkillToolDef` objects:

```json
[
  {
    "name": "run_gh",
    "description": "Execute a GitHub CLI (gh) command.",
    "input_schema": {
      "type": "object",
      "properties": {
        "command": {
          "type": "string",
          "description": "The gh subcommand and arguments to execute"
        }
      },
      "required": ["command"]
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh"
    }
  }
]
```

### SkillToolDef fields

| Field          | Type   | Required | Description                                                |
|----------------|--------|----------|------------------------------------------------------------|
| `name`         | String | Yes      | Tool name (sent to Claude and used for dispatch).          |
| `description`  | String | Yes      | Description shown to Claude explaining what the tool does. |
| `input_schema` | Object | Yes      | JSON Schema object describing the tool's input parameters. |
| `handler`      | Object | Yes      | Dispatch config — see handler variants below.              |

### Handler variants in tools.json

**Exec** — runs a shell command:
```json
{"type": "exec", "command": "handlers/run.sh"}
```
Optional fields: `long_running` (bool), `estimated_duration_secs` (u64).

Exec handlers always capture and return stdout regardless of exit code. Non-zero exits are **not** treated as tool errors — the output includes an `Exit code: N` prefix and the agent interprets the exit code contextually. Only OS-level failures (command not found, timeout) produce tool errors. The `__mika_v1` image envelope protocol is only parsed on exit 0.

**Http** — POSTs to a URL:
```json
{"type": "http", "url": "http://localhost:8080/tools", "method": "POST"}
```
`method` defaults to `"POST"` if omitted.

**Builtin** — calls a compiled Rust function:
```json
{"type": "builtin", "function": "web_search"}
```

Tool definitions are loaded at startup during the skills scan. A restart is required for changes to take effect.

---

## Built-in Skills Reference

These skills are bundled into the binary and seeded into `~/.mika/skills/` on every startup (overwriting existing files unless `MIKA_DISABLE_BUNDLED_SKILLS=true`):

### Builtin-handler skills (always-on)

| Skill      | Keywords                                                       | Tools                                                             | Prompt Snippet |
|------------|----------------------------------------------------------------|-------------------------------------------------------------------|----------------|
| memory     | remember, memory, fact, person, commitment, preference, event  | `update_core_memory`, `store_fact`, `search_memory`, `update_fact` | Yes            |
| reminders  | remind, reminder, schedule, alarm, alert                       | `create_reminder`, `list_reminders`, `cancel_reminder`             | Yes            |
| messaging  | send, message, notify                                          | `send_message`                                                     | No (empty)     |

These three use the `builtin` handler type, so their tools are dispatched through the Rust `ToolRegistry` with full access to the database and agent context. All three are `always_on = true`, meaning they are active on every turn regardless of message content.

### Exec-handler skills

| Skill          | Always On | Keywords                                                                                          | Tools        | Prompt Snippet |
|----------------|-----------|---------------------------------------------------------------------------------------------------|--------------|----------------|
| file-reader    | Yes       | read, file, open, show, cat, view, look at, display, print, content, what does, what's in         | `read_file`  | Yes            |
| shell-exec     | No        | run command, execute, shell, terminal, bash                                                       | `shell_exec` | Yes            |
| tmux           | No        | tmux, terminal, session, pane, window                                                             | `tmux`       | Yes            |
| web-search     | Yes       | search, look up, find out, google, browse, web                                                    | `web_search` | Yes            |
| github         | No        | github, pull request, open pr, my prs, merge pr, close pr, check pr, view pr, pr status, create issue, file an issue, github actions, ci checks, ci pipeline, build status, label, labels, add label, create label, edit label, delete label, remove label | `run_gh`     | Yes            |

### Builtin-handler skills (keyword-triggered)

| Skill              | Keywords                                                                                           | Tools          | Prompt Snippet |
|--------------------|----------------------------------------------------------------------------------------------------|----------------|----------------|
| git-ops            | rebase, merge main, sync main, git sync, sync branch, fast-forward, git fetch, rebase onto, git rebase, git merge, git pull, pull main, checkout, switch branch, git checkout, git switch, worktree, git worktree | `git_ops`  | Yes            |
| google-workspace   | google, gmail, google calendar, google drive, gdrive                                               | `run_gws`      | Yes            |
| skill-review       | review skill, adapt skill, generate variant, tune prompt, skill variant                            | `review_skill` | Yes            |

The **git-ops** skill provides `git_ops` for structured git operations (fetch, rebase, merge, pull, checkout, worktree management). It uses `tokio::process::Command` with `GIT_TERMINAL_PROMPT=0` and scrubs `MIKA_*` env vars from child processes. Supported operations: `fetch` (download remote refs), `rebase` (fetch + rebase onto base ref, auto-aborts on conflict), `merge` (fetch + fast-forward only merge), `pull` (fetch + fast-forward merge in one call), `checkout` (switch branches via `git switch`), `worktree_add` (create worktree with new or existing branch), `worktree_remove` (force-remove worktree), `worktree_list` (porcelain listing), `worktree_prune` (clean stale refs). Optional `push: true` on rebase uses `--force-with-lease` with branch protection (refuses push to `main`/`master`). Parameters: `repo_path` (required, absolute), `base` (default `origin/main`), `push` (rebase only), `branch` (required for checkout/worktree_add), `path` (required absolute for worktree_add/worktree_remove). Pre-flight checks verify the repo path, clean working tree (for rebase/merge/pull), and no in-progress rebase/merge. `timeout_secs = 120` for large repo operations.

The **google-workspace** skill provides `run_gws` for interacting with Google Workspace (Gmail, Calendar, Drive) via the `gws` CLI. It uses a service allowlist (`gmail`, `calendar`, `drive`) and blocks credential/config-smuggling flags (`--token`, `--credentials-file`, `--config`, `--config-dir` including `--flag=value` forms). Scrubs `MIKA_*` env vars from child processes. Uses `gws`'s native keyring-based authentication (set up via `gws auth login`). Requires `gws` CLI installed (included in Docker image). `timeout_secs = 45` to accommodate first-call API schema discovery.

The **skill-review** skill provides `review_skill` for generating and persisting model-tuned `system_prompt.md` variants in a single atomic tool. With no `content`, it reads a skill's root prompt and tool signatures, resolves the agent's current provider/model (extracting canonical tuples for aggregator providers like OpenRouter), and returns structured data so the agent can draft an adapted prompt. With `content`, the same call writes the variant to `generated/<provider>/<sanitized_model>/system_prompt.md` — the path is computed entirely from the runtime context, not user input. Supports single-skill review, batch mode (`skill_name = "*"`, inspect only), dry-run preview, and force overwrite. Linked skills (`--link`) are reviewed normally — the tool emits a `linked: true` warning and any persist call writes through the symlink to the source directory. See the [Model Tuning](#model-tuning) section for details. `timeout_secs = 60`.

The **file-reader** skill (`always_on = true`) provides the `read_file` tool on every turn. It detects image files (JPEG, PNG, GIF, WebP) via `file --mime-type` and returns them using the `__mika_v1` envelope protocol for visual analysis by the agent, rather than dumping raw binary to stdout. Being always-on ensures `read_file` is available for image chaining (e.g., a screenshot skill saves a file, then the agent uses `read_file` to view it).

The **github** skill provides `run_gh` for interacting with GitHub via the `gh` CLI. It uses an allowlist of safe subcommands (pr, issue, run, workflow, release, repo, search, label, api) and scrubs sensitive `MIKA_*` environment variables before execution. The `api` subcommand enables REST and GraphQL operations (e.g., closing milestones via `gh api --method PATCH`); each `gh api` invocation emits a `gh_api_invocation` structured log event with `session_id`, `method`, and `path` fields for post-hoc observability. Requires `gh` CLI to be installed (included in Docker image).

All bundled exec-handler scripts require `jq` for JSON input parsing and will fail with a clear error if `jq` is not found. The Docker agent image includes `jq`; CLI users must install it separately. Note: all exec-handler skills are excluded from heartbeat mode by `safe_always_on_skills()`.

### Prompt-only skills (no tools)

| Skill           | Keywords                                                      | Prompt Snippet |
|-----------------|---------------------------------------------------------------|----------------|
| self-knowledge  | help, what can you do, capabilities, commands, how to use     | Yes            |
| mcp             | mcp, model context protocol, mcp server, mcp tool              | Yes            |
| browser-control | playwright, browse to, web page, navigate to, take screenshot, browser automation, fill form, web scraping | Yes |
| agents-teams    | delegate, delegate task, run team, list agents, list teams, team workflow, team status, team history, multi-agent | Yes |

These skills provide only system prompt guidance — they have no tools of their own. The **self-knowledge** skill (`always_on = true`) instructs the agent to use `get_documentation` before answering questions about its systems (architecture, CLI, API, skills, etc.) and to check its home directory files (`list_agent_files`, `read_agent_file`) before answering questions about its own configuration or internals (soul.md, identity.toml, mcp.json, installed skills). The **mcp** skill explains how to configure external MCP servers via `mcp.json` and the `mika mcp` CLI commands. The **browser-control** skill guides the agent on using browser automation tools from a Playwright MCP server — snapshot-then-act workflow, ref-based interaction, step budgeting, and security boundaries (no credentials in tool params, no `file://` URLs, no internal network access). See [Browser Control](browser-control.md) for setup instructions. The **agents-teams** skill provides behavioral guidance for using the 6 management tools (`delegate_task`, `run_team`, `list_agents`, `list_teams`, `get_team_status`, `get_team_history`) — when to delegate vs run a team, delegate limitations, and timeout expectations.

---

## Startup Validation

At agent startup, Mika runs `validate_skill()` on every loaded skill **after** applying database overrides (`apply_overrides()`). This catches semantic issues that the initial structural scan (`scan_skills_dir()`) does not — deprecated manifest sections, missing handler scripts, placeholder mismatches, and more.

### Decision Matrix

| Diagnostic | Action | Rationale |
|-----------|--------|-----------|
| Missing handler script | **Skip** | Skill cannot execute tool calls |
| Handler not executable | **Skip** | Will fail at runtime |
| Oversized/invalid/unreadable tools.json | **Skip** | Tools won't load |
| Unreadable skill.toml (symlink race) | **Skip** | Manifest disappeared after initial scan |
| `[llm]` section in skill.toml | **Warn** | Runtime ignores it; use `mika skills llm` instead |
| Skill name in keywords | **Warn** | Cosmetic — redundant but harmless |
| Invalid context type | **Warn** | Context injection fails gracefully |
| Placeholder mismatch | **Warn** | Context may not render correctly |
| Invalid markdown in system_prompt.md | **Warn** | Prompt loads but may have formatting issues |

**Catch-all:** If `validate_skill()` returns zero Ok diagnostics and at least one Fail, the skill is treated as skip-worthy regardless of the specific failure type.

### How Warnings Surface

| Mode | How warnings appear |
|------|-------------------|
| **TUI** (`mika`) | `ChatRole::System` message at startup listing up to 5 skills with warnings |
| **`mika ask`** | Summary printed to stderr |
| **Server** (`mika-server`) | `tracing::warn` log entries with structured fields (`skill`, `error_kind`, `message`) |

For full diagnostic output, run `mika skills validate`.

---

## Marketplace (Installing Community Skills)

Mika supports installing skills from Git repositories. This lets the community share skills without any central infrastructure — just push a skill to a Git repo and share the URL.

### Installing a Skill

```bash
# From GitHub shorthand
mika skills install user/repo

# From full URL
mika skills install https://github.com/user/mika-skill-weather.git

# Install under a different name (alias)
mika skills install user/repo --name my-weather

# From a local path (snapshot copy)
mika skills install /path/to/my-skill
mika skills install file:///path/to/my-skill

# From a local path (live symlink — changes reflected immediately)
mika skills install /path/to/my-skill --link

# Local repo root with multiple skills (interactive picker)
mika skills install /path/to/mika-skills --link
```

**Git sources:** The install process clones the repository (shallow clone), scans for `skill.toml` files (up to 2 levels deep), presents an interactive picker if multiple skills are found, validates the manifest and checks for name collisions, copies the skill directory into `~/.mika/skills/<name>/`, and records the installation in `marketplace.lock`.

**Local sources:** Accepts absolute paths or `file://` URIs. Without `--link`, files are copied (snapshot). With `--link`, a symlink is created so changes to the source directory are reflected immediately — ideal for skill development. `--link` is not supported with git sources.

Skills with exec handlers show a security warning before installation and require confirmation to proceed. For `--link` installs, an additional note warns that handler scripts can be modified at any time.

### Updating Skills

```bash
# Update a specific skill
mika skills update weather

# Update all marketplace skills
mika skills update
```

Update behavior depends on the source type:
- **Git sources:** Re-clones the repo and replaces the installed skill with the latest version. The lock file is updated with the new commit hash.
- **Local snapshots:** Re-copies from the original source path. Fails with a clear message if the source directory no longer exists.
- **Linked skills:** No-op — source changes are always current. The update summary reports these as "Linked (no-op)".

### Uninstalling Skills

```bash
mika skills uninstall weather
```

This removes the skill directory and its lock file entry. Built-in skills cannot be uninstalled (use `mika skills disable` instead).

### Skill Origins

Skills have four possible origins, shown in `list_skills` output:

- **[built-in]** — Bundled with Mika, re-synced on startup
- **[marketplace]** — Installed from a Git repository or local path via `mika skills install`
- **[marketplace/linked]** — Installed via `mika skills install --link` (symlink to source)
- **[custom]** — Created locally via `mika skills create` or manually

### Publishing Skills

To publish a Mika skill, create a Git repository with this structure:

**Single-skill repo** (skill.toml at the root):
```
mika-skill-weather/
  skill.toml
  system_prompt.md
  tools.json
  handlers/
    run.sh
  README.md
```

**Multi-skill repo** (multiple skill directories):
```
mika-skills-collection/
  weather/
    skill.toml
    system_prompt.md
  news/
    skill.toml
    tools.json
  README.md
```

The installer scans up to 2 directory levels for `skill.toml` files. Ensure your skill names are valid (alphanumeric, hyphens, underscores only) and don't collide with built-in skill names.

### Lock File

Marketplace installations are tracked in `~/.mika/agents/<agent>/marketplace.lock`:

```toml
# Git source
[skills.weather]
url = "https://github.com/user/mika-skill-weather.git"
path = "."
commit = "abc123def456"
installed_at = "2026-03-02T10:30:00Z"
updated_at = "2026-03-02T10:30:00Z"

# Local snapshot
[skills.my-dev-skill]
url = "file:///home/user/projects/my-skill"
path = "."
commit = ""
installed_at = "2026-03-19T10:00:00Z"
updated_at = "2026-03-19T10:00:00Z"

# Linked (symlink)
[skills.self-dev]
url = "file:///home/user/projects/mika-skills/self-dev"
path = "."
commit = ""
linked = true
installed_at = "2026-03-19T10:00:00Z"
updated_at = "2026-03-19T10:00:00Z"
```

---

## Creating a Custom Skill

This walkthrough creates an exec-based skill that converts between time zones using a shell script. You can also scaffold this with `mika skills create timezone`.

### Step 1: Create the skill directory

```bash
mkdir -p ~/.mika/skills/timezone/handlers
```

### Step 2: Write skill.toml

Create `~/.mika/skills/timezone/skill.toml`:

```toml
[skill]
name = "timezone"
description = "Convert times between time zones"
timeout_secs = 10

[triggers]
keywords = ["timezone", "time zone", "convert time", "what time"]
```

### Step 3: Write tools.json

Create `~/.mika/skills/timezone/tools.json`. Each tool definition includes the handler dispatch config:

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
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh"
    }
  }
]
```

### Step 4: Write the handler script

Create `~/.mika/skills/timezone/handlers/run.sh`:

```bash
#!/bin/sh
set -eu

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required" >&2; exit 1; }

# Tool input JSON is piped via stdin
INPUT=$(cat)

# Parse input fields using jq
TIME=$(printf '%s\n' "$INPUT" | jq -r '.time')
FROM_TZ=$(printf '%s\n' "$INPUT" | jq -r '.from_timezone')
TO_TZ=$(printf '%s\n' "$INPUT" | jq -r '.to_timezone')

# Convert using date command
RESULT=$(TZ="$TO_TZ" date -d "TZ=\"$FROM_TZ\" $TIME" '+%Y-%m-%d %H:%M:%S %Z' 2>&1)

echo "{\"converted_time\": \"$RESULT\", \"from\": \"$FROM_TZ\", \"to\": \"$TO_TZ\"}"
```

Make it executable:

```bash
chmod +x ~/.mika/skills/timezone/handlers/run.sh
```

### Step 5: Write system_prompt.md

Create `~/.mika/skills/timezone/system_prompt.md`:

```markdown
- Use convert_timezone to convert times between time zones when the user asks.
- Always use IANA timezone names (e.g., America/New_York, not EST).
- If the user gives an ambiguous timezone abbreviation, ask for clarification or use the most common interpretation.
```

### Step 6: Validate and test

Validate the skill structure before starting Mika:

```bash
mika skills validate timezone
```

A restart is required for Mika to discover new skill directories, because `skill.toml` manifests are scanned once at startup. After restarting Mika (or the mika-server process), send a message like:

```
What time is it in Tokyo when it's 3 PM in New York?
```

The keyword `"time"` in the message matches `"what time"` from the triggers (substring match), so the timezone skill activates. Claude receives the `convert_timezone` tool definition and the prompt snippet, and can call the tool to answer.

If the skill fails to load, check the Mika logs for warnings or run `mika skills validate`. Common issues:

- Missing `[skill]` section in `skill.toml` (legacy format with `[handler]` section)
- Invalid TOML syntax in `skill.toml`
- Handler script not executable (`chmod +x`)
- `tools.json` missing `handler` field on tool definitions
- `tools.json` not valid JSON or missing required fields

---

## Customizing Built-in Skills

Bundled skills are re-synced from compiled-in templates on every startup, ensuring updates propagate to existing installations. To preserve local edits to handler scripts, set `MIKA_DISABLE_BUNDLED_SKILLS=true` (not recommended in production).

### Example: Disable always-on for reminders

For built-in skills, use the `update_skill` tool (which persists the override in the database, surviving restarts):

```
update_skill(name: "reminders", always_on: false)
```

The override is stored in the `skill_overrides` table and applied automatically on startup. Setting `always_on` back to the bundled default (`true` for reminders) removes the override row. The `list_skills` tool shows an `[override]` badge when the effective value differs from the bundled default.

> **Note:** Directly editing `skill.toml` for built-in skills will not persist — `seed_bundled_skills()` overwrites it on every startup.

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

### Per-skill LLM override (v20+)

A skill's `[llm]` section in `skill.toml` captures the **author's intent** — the
provider/model the skill was designed and validated for. To change which model
a specific skill uses **without editing committed files**, use the per-agent
DB override layer (schema v20):

```bash
mika skills llm qa-review set anthropic/claude-sonnet-4-6
mika skills llm qa-review show     # → [db-override]
mika skills llm qa-review reset    # clear override
```

Resolution order at runtime:

1. **DB override** (`skill_overrides.llm_provider` / `llm_model`) — set via the
   CLI above.
2. **Agent default** — the agent's active provider and model.

Note: The `[llm]` section in `skill.toml` is no longer supported (#504). All
LLM overrides are DB-only. `validate_skill()` rejects skill.toml files
containing `[llm]`.

`mika skills llm <name> show` annotates the source of the effective value:
`[db-override]` or `[agent-default]`.

### Persistent overrides for built-in skills (v7+)

Built-in skill `always_on` preferences are stored in the SQLite `skill_overrides` table (schema v7). This table survives `seed_bundled_skills()` re-sync cycles, which overwrite `skill.toml` on every startup.

**How it works:**

1. `update_skill` detects whether a skill is built-in or custom/marketplace
2. For built-in skills, `always_on` changes are written to the DB via `set_skill_override()`
3. For custom/marketplace skills, `always_on` is written to `skill.toml` as before
4. After `scan_skills_dir()` loads manifests from disk, `SkillRegistry::apply_overrides()` applies DB overrides
5. Optionally, `apply_transient_disable()` and `apply_transient_always_on()` apply CLI `--disable-skill` / `--enable-skill` overrides (runtime-only, not persisted)
6. Setting `always_on` back to the bundled default automatically deletes the override row (prevents stale overrides from blocking future bundled default changes)
7. `delete_skill` and `mika skills uninstall` clean up override rows

**CLI transient override:**

`mika ask --enable-skill <name>` forces a skill to `always_on` for a single invocation without touching the database. Repeatable for multiple skills: `--enable-skill self-dev --enable-skill qa-review`. Cannot resurrect disabled or skipped skills — emits a warning instead.

`mika ask --disable-skill <name>` transiently evicts a skill from the registry for a single invocation. Repeatable for multiple skills: `--disable-skill self-dev --disable-skill qa-review`. Useful for interactive sessions where an `always_on` skill is not needed. A skill cannot appear in both `--enable-skill` and `--disable-skill` in the same invocation (hard error).

**Viewing overrides:**

The `list_skills` tool shows an `[override]` badge when the effective `always_on` value differs from the bundled default:

```
- web-search (enabled) [built-in] — Search the web [always-on] [override]
```

---

## Model Tuning

Mika supports per-provider/model prompt variants so each skill can have a model-tuned `system_prompt.md`. The built-in **skill-review** skill automates generating these variants.

### How it works

The `skill-review` skill reads a skill's root `system_prompt.md` and tool signatures, then lets the agent's LLM generate an adapted prompt optimized for the current model. The result is written to the skill's variant directory.

### Invoking skill-review

Use natural language with any of these keywords: "review skill", "adapt skill", "generate variant", "tune prompt", "skill variant".

**Examples:**

```
review the web-search skill
adapt the shell-exec skill prompt for this model
generate variant for all skills
```

### The review_skill tool

The skill exposes one tool:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `skill_name` | string | (required) | Skill name, or `*` for all skills |
| `content` | string | (omit to inspect) | Full adapted prompt to persist as the model-tuned variant |
| `dry_run` | boolean | false | Show adapted prompt without writing |
| `force` | boolean | false | Overwrite existing variants |

### Where variants are stored

Variants follow the two-level resolution hierarchy:

```
~/.mika/agents/<name>/skills/<skill>/
  system_prompt.md                           # Root prompt (fallback)
  anthropic/
    claude-sonnet-4-6/
      system_prompt.md                       # Model-specific variant
  openai/
    gpt-4o/
      system_prompt.md                       # Another variant
```

**Resolution order:** model-specific prompt -> root prompt. There are no provider-level prompts (models from the same provider have different requirements).

### Aggregator providers

When running on an aggregator provider like OpenRouter (model name `anthropic/claude-sonnet-4`), skill-review extracts the canonical provider and model:

- OpenRouter `anthropic/claude-sonnet-4` -> writes to `anthropic/claude-sonnet-4/system_prompt.md`
- OpenRouter `openai/gpt-4o` -> writes to `openai/gpt-4o/system_prompt.md`

This means variants generated via OpenRouter are used when the agent later runs on the native provider directly.

### Batch mode

Use `skill_name = "*"` to review all skills. The tool returns a list of eligible and skipped skills. Due to the agent's step limit, batch mode processes skills iteratively — you may need multiple invocations for large skill sets.

Skills are skipped in batch mode when:
- **Trust-critical** — skills governing security, identity, or orchestration cannot be reviewed (#486)
- No `system_prompt.md` (nothing to adapt)
- Variant already exists (unless `force = true`)

Trust-critical skills (skill-review, self-knowledge, agents-teams) are also rejected in single-skill mode — the tool returns a clear message and does not touch the filesystem. All other bundled skills (tmux, shell-exec, web-search, file-reader, git-ops, google-workspace, github, mcp, browser-control) are reviewable — their prompts focus on tool usage mechanics and are safe to adapt per-model.

Linked skills are **not** skipped — they are reviewed and can have variants
written through to their source directory (with a structured warning).

### Dry-run workflow

Set `dry_run = true` to preview the adaptation without writing:

```
review skill web-search with dry_run
```

The agent will generate and display the adapted prompt for review. You can then decide whether to proceed with writing.

### Manual editing

Variant files are plain markdown — you can edit them directly:

```bash
# Edit a variant
vim ~/.mika/agents/<name>/skills/web-search/anthropic/claude-sonnet-4-6/system_prompt.md

# Remove a variant (falls back to root prompt)
rm -rf ~/.mika/agents/<name>/skills/web-search/anthropic/claude-sonnet-4-6/
```

### Linked skills

Linked skills (installed with `--link`) cannot have variants written to them because the skill directory is a symlink to the author's source directory. Unlink first, then generate variants:

```bash
mika skills uninstall my-skill
mika skills install /path/to/my-skill  # Without --link
```

---

## Security Considerations

### Exec handlers run unsandboxed

Exec handlers spawn a child process with `tokio::process::Command`. The process runs with the same user permissions as Mika itself. There is no sandboxing, chroot, or capability restriction.

**Environment variable scrubbing:** The exec handler executor scrubs all `MIKA_*` environment variables from child processes before spawning them. This applies to all exec-handler skills (built-in, marketplace, and custom) and prevents API keys from leaking to handler scripts. Bundled handler scripts (shell-exec, github) additionally `unset` specific vars in their scripts as defense-in-depth.

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

The `list_skills` agent tool runs `apply_load_safety_check()` and reports the skipped count in its output. When skills were skipped, a warning footer is appended: `Warning: N skill(s) skipped due to errors. Run 'mika skills validate' for details.` This enables agents to self-diagnose a degraded skill registry. When no skills are skipped, the output is unchanged.

## QA Verdict Contract

mika-qa-bot posts PR verdicts as GitHub reviews with:

| Verdict | Review state | Routing |
| --- | --- | --- |
| `pass` | `APPROVED` | Satisfies branch protection's "1 approval required" gate (per mika-skills#55, 2026-03-30) so `pr_merge_with_gate` (mika-skills#119, 2026-04-11) clears without manual operator clicks. |
| `hold[*]`, `block[*]` | `COMMENTED` | Stays advisory; preserves the operator's "merge anyway" escape hatch. |
| **never** | `CHANGES_REQUESTED` | Forbidden — it conflates advisory verdicts with GitHub's review-required gate (mika#487 invariant). |

- **Body:** contains a `VERDICT: <class>[<detail>]` token (e.g., `VERDICT: pass`, `VERDICT: hold[review]`, `VERDICT: block[tests]`)

**The `state` field is NOT authoritative. The `VERDICT:` token in the body is.**

QA is advisory for hold/block verdicts — it never uses `CHANGES_REQUESTED` to block GitHub's native merge button. Using `CHANGES_REQUESTED` would conflate advisory verdicts with GitHub's review-required gate and is explicitly rejected.

### BAD — gating on state

```rust
// ❌ state alone cannot distinguish qa-bot routing outcomes safely
if review.state == "CHANGES_REQUESTED" { retry() }
```

### GOOD — parsing the token

```rust
// ✅ parse the VERDICT: token from the review body
if body.contains("VERDICT: hold") || body.contains("VERDICT: block") { retry() }
```

Any webhook filter, routing rule, or verdict parser that gates on `state` instead of body content is a bug. See issue [#487](https://github.com/senara-solutions/mika/issues/487) for the incident that motivated this contract.
