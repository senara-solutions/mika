# Configuration Reference

This document covers all configuration options for Mika, including the agent
(CLI and server modes) and the gateway.

---

## Directory Layout

When Mika runs for the first time, it bootstraps the home directory at
`~/.mika/` with the following structure:

```
~/.mika/
  config.toml        # User configuration (cascade layer 3)
  identity.toml      # Assistant name and emoji
  soul.md            # Personality definition (system prompt)
  heartbeat.md       # Heartbeat checklist for proactive behaviors
  user.md            # User self-description (seeds initial context)
  data/
    mika.db          # SQLite database (conversations, memory, reminders)
  logs/              # Log files
  skills/            # Skill definitions (one subdirectory per skill)
    memory/
      skill.toml
      system_prompt.md
    reminders/
      skill.toml
      system_prompt.md
    messaging/
      skill.toml
      system_prompt.md
```

On Unix systems, directories are set to `0700` and files to `0600` to protect
sensitive data.

Bootstrap never overwrites existing files. If a file already exists, the default
content is skipped. This means you can safely customize any file and re-run
`mika setup` without losing changes.

---

## Configuration Cascade

Settings are loaded from four sources, in order of increasing priority. A value
set at a higher layer overrides the same value from a lower layer.

| Priority | Source                    | Description                                |
|----------|---------------------------|--------------------------------------------|
| 1 (lowest) | `config/default.toml`   | Bundled defaults, relative to working directory |
| 2        | `config/local.toml`       | Gitignored local overrides for development |
| 3        | `~/.mika/config.toml`    | User home directory config                 |
| 4 (highest) | `MIKA_*` env vars      | Environment variables, highest priority    |

All config files are optional. If a file does not exist, it is silently skipped.

### Example: Override model in home config

In `~/.mika/config.toml`:

```toml
claude_model = "claude-opus-4-6"
log_level = "debug"
```

### Example: Override model via environment variable

```sh
export MIKA_CLAUDE_MODEL=claude-haiku-4-5
```

The environment variable takes precedence over all config files.

---

## Settings Reference

Complete table of all `Settings` struct fields for the agent (CLI and server modes).

| Field | Type | Default | Env Var | Description |
|-------|------|---------|---------|-------------|
| `anthropic_api_key` | `Option<String>` | None | `MIKA_ANTHROPIC_API_KEY` | Anthropic API key or OAuth subscription token. Auto-detected from prefix (`sk-ant-oat` = OAuth, otherwise = API key). Required for any command that calls the Claude API. |
| `claude_model` | `String` | `claude-sonnet-4-6` | `MIKA_CLAUDE_MODEL` | Claude model ID to use for inference. |
| `claude_max_tokens` | `u32` | `4096` | `MIKA_CLAUDE_MAX_TOKENS` | Maximum tokens for Claude responses. |
| `db_path` | `PathBuf` | `~/.mika/data/mika.db` | `MIKA_DB_PATH` | Path to the SQLite database file. If not explicitly set, resolves to `{home_dir}/data/mika.db`. |
| `log_level` | `String` | `info` | `MIKA_LOG_LEVEL` | Log level filter. Valid values: `trace`, `debug`, `info`, `warn`, `error`. |
| `routing_url` | `Option<String>` | None | `MIKA_ROUTING_URL` | Gateway URL for outbound message delivery. Required in server mode. |
| `customer_id` | `Option<String>` | None | `MIKA_CUSTOMER_ID` | Customer identifier. Set per container in Kubernetes deployments. |
| `server_port` | `u16` | `8080` | `MIKA_SERVER_PORT` | HTTP server listen port. Only used in server mode (`mika-server`). |
| `openai_api_key` | `Option<String>` | None | `MIKA_OPENAI_API_KEY` | OpenAI API key for embedding generation. Enables vector similarity in Layer 3 hybrid search. |
| `embedding_model` | `String` | `text-embedding-3-small` | `MIKA_EMBEDDING_MODEL` | OpenAI embedding model ID. |
| `embedding_dimensions` | `u32` | `512` | `MIKA_EMBEDDING_DIMENSIONS` | Embedding vector dimensions. |
| `brave_api_key` | `Option<String>` | None | `MIKA_BRAVE_API_KEY` | Brave Search API key for `web_search` builtin skill. Get a free key at https://brave.com/search/api/. |
| `internal_token` | `Option<SecretString>` | None | `MIKA_INTERNAL_TOKEN` | Shared bearer token for gateway-to-container auth. Must be exactly 64 hex characters (32 bytes hex-encoded). Required in server mode. |
| `server_log_file` | `Option<PathBuf>` | None | `MIKA_SERVER_LOG_FILE` | File path for mika-server log output. Logs go to stdout + file when set. |
| `disable_bundled_skills` | `bool` | `false` | `MIKA_DISABLE_BUNDLED_SKILLS` | Skip bundled skill re-sync on startup. Useful for debugging handler scripts. **Do not enable in production** — prevents security updates to handler scripts from propagating. |

The `home_dir` field is also present on the struct but is not configurable via
file or environment variable. It is resolved automatically from `$MIKA_HOME` or
defaults to `~/.mika/`.

### Security notes

- `anthropic_api_key`, `internal_token`, and `brave_api_key` are redacted in
  `Debug` output (printed as `[REDACTED]`). The `mika config` command
  distinguishes between credential types: `OAuth token [REDACTED]` or
  `API key [REDACTED]`.
- `anthropic_api_key` accepts both standard API keys and OAuth subscription
  tokens. Mika detects the type from the `sk-ant-oat` prefix and adjusts the
  HTTP auth scheme automatically (Bearer + `anthropic-beta` header for OAuth,
  `x-api-key` header for standard keys).
- Secrets should be set via environment variables, never committed to config files.
- `internal_token` is validated on load: if present, it must be exactly 64
  hex characters. Invalid values cause an immediate startup error.

---

## identity.toml

Defines the assistant's display identity.

**Default content:**

```toml
name = "Mika"
emoji = "✦"
```

| Field | Description |
|-------|-------------|
| `name` | The assistant's display name, used in prompts and UI. |
| `emoji` | A single character or emoji shown alongside the assistant's name in the TUI. |

To customize, edit `~/.mika/identity.toml`:

```toml
name = "Jarvis"
emoji = ">"
```

---

## soul.md

The personality definition file. Its entire content is injected into the system
prompt for every conversation, defining how the assistant communicates and
behaves.

**Default content:**

```markdown
# Mika -- Executive Assistant

## Personality
You are Mika, a senior executive assistant. You are calm, confident,
and concise. You anticipate needs rather than wait for instructions.
You protect the user's time fiercely.

## Communication style
- Lead with the answer, then context if needed
- Never say "I hope this helps" or "Let me know if you need anything"
- Match the user's energy -- brief if they're brief, detailed if they ask
- Use their first name naturally, not every message
- Push back respectfully when something doesn't make sense

## Proactive behaviors
- Flag scheduling conflicts before they happen
- Remind about commitments approaching their deadline
- Surface patterns ("You've rescheduled this meeting 3 times -- want to cancel it?")

## Boundaries
- Never pretend to have done something you haven't
- Say "I don't know" when you don't know
- Ask for clarification rather than guess on high-stakes decisions
```

To customize, edit `~/.mika/soul.md`. You can completely rewrite the personality,
add domain-specific instructions, or adjust the communication style. The file is
read on each agent loop start, so changes take effect on the next conversation.

---

## heartbeat.md

The heartbeat checklist defines what the agent reviews during periodic background
heartbeat cycles. In server mode, heartbeats are triggered by a Kubernetes
CronJob hitting the `/heartbeat` endpoint. The agent reads this file and uses it
as guidance for proactive check-ins.

**Default content:**

```markdown
# Heartbeat Checklist

- Review active commitments approaching deadline
- Check if any meetings are coming up in the next 2 hours
- Look for stale priorities (no updates in 3+ days)
- Surface patterns worth mentioning
```

Heartbeats are subject to pre-filters: active hours (8:00-21:00 local time),
rate limits (1 per hour, 3 per day), and suppression if the user messaged within
2 hours.

---

## user.md

A free-form file where the user describes themselves. This content seeds Mika's
initial understanding during onboarding and is included in context when relevant.

**Default content:**

```markdown
# Tell Mika about yourself

Edit this file with your name, role, preferences, and anything
you'd like Mika to know about you. This seeds Mika's initial
understanding when starting fresh.
```

Fill in details like:

```markdown
# About me

Name: Alex Chen
Role: VP Engineering at Acme Corp
Timezone: US/Pacific
Preferences: I prefer bullet points over paragraphs. Don't schedule
anything before 9am. Always CC my admin (admin@acme.com) on meeting changes.
```

---

## Environment Variables

All environment variables use the `MIKA_` prefix. The separator for nested
fields is `__` (double underscore).

### CLI mode

For running `mika` (the TUI chat client), only the API key is required:

| Variable | Required | Description |
|----------|----------|-------------|
| `MIKA_ANTHROPIC_API_KEY` | Yes | Anthropic API key or OAuth subscription token |
| `MIKA_CLAUDE_MODEL` | No | Override model (default: `claude-sonnet-4-6`) |
| `MIKA_CLAUDE_MAX_TOKENS` | No | Override max tokens (default: `4096`) |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level (default: `info`) |
| `MIKA_HOME` | No | Override home directory (default: `~/.mika/`) |
| `MIKA_OPENAI_API_KEY` | No | OpenAI API key for Layer 3 vector search |
| `MIKA_BRAVE_API_KEY` | No | Brave Search API key for web search skill |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |

### Server mode

For running `mika-server` (the HTTP server in Kubernetes), additional variables
are required for inter-service communication:

| Variable | Required | Description |
|----------|----------|-------------|
| `MIKA_ANTHROPIC_API_KEY` | Yes | Anthropic API key or OAuth subscription token |
| `MIKA_ROUTING_URL` | Yes | Gateway URL for outbound message delivery |
| `MIKA_INTERNAL_TOKEN` | Yes | Shared bearer token (64 hex chars) for gateway auth |
| `MIKA_CUSTOMER_ID` | Yes | Customer identifier for this container |
| `MIKA_SERVER_PORT` | No | Listen port (default: `8080`) |
| `MIKA_CLAUDE_MODEL` | No | Override model |
| `MIKA_CLAUDE_MAX_TOKENS` | No | Override max tokens |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |

### Gateway

The gateway (`mika-gateway`) uses environment variables exclusively -- it does
not read config files. All fields are loaded from `MIKA_*` environment variables.

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `MIKA_DATABASE_URL` | Yes | -- | Postgres connection string |
| `MIKA_TELEGRAM_BOT_TOKEN` | Yes | -- | Telegram Bot API token |
| `MIKA_TELEGRAM_WEBHOOK_SECRET` | Yes | -- | Secret for validating inbound Telegram webhooks (64 hex chars) |
| `MIKA_TELEGRAM_WEBHOOK_URL` | Yes | -- | Public URL Telegram calls for webhook delivery |
| `MIKA_INTERNAL_TOKEN` | Yes | -- | Shared bearer token for gateway-to-container auth (64 hex chars) |
| `MIKA_GATEWAY_PORT` | No | `8080` | Listen port |
| `MIKA_LOG_LEVEL` | No | `info` | Log level |

Both `MIKA_INTERNAL_TOKEN` and `MIKA_TELEGRAM_WEBHOOK_SECRET` are validated on
startup: each must be exactly 64 hexadecimal characters (32 bytes hex-encoded).
Invalid values cause an immediate startup error.

### Generating tokens

Use `openssl` to generate compliant 64-character hex tokens:

```sh
openssl rand -hex 32
```

This produces exactly 64 hex characters suitable for `MIKA_INTERNAL_TOKEN` and
`MIKA_TELEGRAM_WEBHOOK_SECRET`.

---

## Model Configuration

Mika works with any Claude model ID. The following models have been tested:

| Model ID | Description |
|----------|-------------|
| `claude-sonnet-4-6` | Default. Good balance of speed and quality. |
| `claude-opus-4-6` | Highest quality. Slower and more expensive. |
| `claude-haiku-4-5` | Fastest and cheapest. Suitable for simple tasks. |

### Switching models

**Via config file** (`~/.mika/config.toml`):

```toml
claude_model = "claude-opus-4-6"
```

**Via environment variable:**

```sh
export MIKA_CLAUDE_MODEL=claude-opus-4-6
```

**Via bundled config** (`config/default.toml`):

```toml
claude_model = "claude-opus-4-6"
claude_max_tokens = 4096
```

The environment variable always wins if set, regardless of config file values.

---

## Advanced: $MIKA_HOME

The `MIKA_HOME` environment variable overrides the default home directory
location (`~/.mika/`). This is useful for:

- Running multiple isolated Mika instances on the same machine
- Testing with a temporary home directory
- Containerized deployments where the home path is mounted differently

### Resolution order

1. If `$MIKA_HOME` is set, use its value as the home directory.
2. Otherwise, use `~/.mika/` (the `.mika` subdirectory of the user's OS home
   directory, resolved via the `dirs` crate).

### Example: Isolated test instance

```sh
export MIKA_HOME=/tmp/mika-test
mika setup   # Bootstraps /tmp/mika-test/ with default files
mika          # Uses /tmp/mika-test/ for all data
```

### Example: Multiple profiles

```sh
# Work profile
MIKA_HOME=~/.mika-work mika

# Personal profile
MIKA_HOME=~/.mika-personal mika
```

Each profile gets its own database, configuration, personality files, and
skills directory. They are completely independent.

### Effect on db_path

When `db_path` is not explicitly set (via config file or `MIKA_DB_PATH`), it
defaults to `{home_dir}/data/mika.db`. Setting `MIKA_HOME` changes the home
directory, which in turn changes the default database location. If `db_path` is
explicitly set to an absolute path, `MIKA_HOME` has no effect on the database
location.
