# Configuration Reference

This document covers all configuration options for Mika, including the agent
(CLI and server modes) and the gateway.

---

## Directory Layout

When Mika runs for the first time, it bootstraps the home directory at
`~/.mika/` with the following structure:

```
~/.mika/
  .env               # Secrets (API keys, tokens) — auto-loaded, 0600 perms
  config.toml        # User configuration (non-secret settings)
  identity.toml      # Assistant name and emoji
  soul.md            # Personality definition (system prompt)
  heartbeat.md       # Heartbeat checklist for proactive behaviors
  user.md            # User self-description (seeds initial context)
  mcp.json           # MCP server configuration (optional, see below)
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

### mcp.json

MCP (Model Context Protocol) servers are configured in `{agent_home}/mcp.json`.
This file is not bootstrapped automatically -- create it via `mika mcp add` or
by writing the file manually.

```json
{
  "mcpServers": {
    "filesystem": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
      "env": {},
      "enabled": true
    },
    "remote-api": {
      "transport": "http",
      "url": "https://mcp.example.com/v1",
      "headers": {
        "Authorization": "Bearer sk-my-token",
        "X-Api-Key": "my-api-key"
      },
      "enabled": true
    }
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `transport` | `"stdio"` or `"http"` | Yes | Transport type |
| `command` | String | stdio only | Command to run as child process |
| `args` | String array | No | Arguments for the command |
| `env` | Object | No | Environment variables for stdio child process |
| `url` | String | http only | URL of the remote MCP server |
| `headers` | Object | No | HTTP headers for http transport (e.g. Authorization) |
| `enabled` | Boolean | No | Default `true`. Set to `false` to disable without removing. |

**CLI management:**

- `mika mcp list` -- Show configured servers, status, and header keys
- `mika mcp add <name> --transport stdio --command <cmd> [--args ...]`
- `mika mcp add <name> --transport http --url <url> [--header KEY=VALUE ...]`
- `mika mcp remove <name>` -- Remove a server
- `mika mcp enable <name>` -- Enable a disabled server
- `mika mcp disable <name>` -- Disable without removing

**Security:** `mcp.json` is written with `0600` permissions on Unix. Header and
env values are redacted in debug output. Header values passed via `--header` on
the command line are visible in shell history and process listings -- for
sensitive tokens, edit `mcp.json` directly.

---

## Configuration Cascade

Settings are loaded from four sources, in order of increasing priority. A value
set at a higher layer overrides the same value from a lower layer.

| Priority | Source                    | Description                                |
|----------|---------------------------|--------------------------------------------|
| 1 (lowest) | Rust defaults           | Compiled-in serde defaults (e.g. `claude-sonnet-4-6`) |
| 2        | TOML config files         | `~/.mika/config.toml` + optional `~/.mika/agents/X/config.toml` |
| 3        | `~/.mika/.env`            | Secrets file (API keys, tokens) — loaded via dotenvy |
| 4 (highest) | `MIKA_*` env vars      | Shell environment variables, always win    |

All config files are optional. If a file does not exist, it is silently skipped.

### Secrets in `~/.mika/.env`

Store API keys and tokens in `~/.mika/.env` instead of config files or shell
profiles. This file is auto-loaded on startup and does **not** override
existing shell environment variables. File permissions are set to `0600`.

```sh
# ~/.mika/.env
MIKA_LLM_API_KEY=sk-ant-api03-...
MIKA_OPENAI_API_KEY=sk-...
MIKA_BRAVE_API_KEY=BSA...
MIKA_INVESTIGATE_GITHUB_TOKEN=ghp_...
MIKA_GITHUB_REPO=owner/repo
```

Run `mika setup` to interactively configure secrets (API keys, tokens) and
preferences (telemetry) — secrets are written to `~/.mika/.env`, config to
`~/.mika/config.toml`. The wizard auto-generates `MIKA_INTERNAL_TOKEN` for
server mode.

### GitHub issue creation (dashboard investigation)

The investigation panel can create GitHub issues when both `MIKA_INVESTIGATE_GITHUB_TOKEN`
and `MIKA_GITHUB_REPO` are set. Steps:

1. Create a GitHub Personal Access Token:
   - **Fine-grained token** (recommended): Settings → Developer settings →
     Fine-grained tokens → select your repo → Permissions → Issues: Read and Write
   - **Classic token**: select `repo` scope (private repos) or `public_repo`
     (public repos)
2. Add to `~/.mika/.env`:
   ```sh
   MIKA_INVESTIGATE_GITHUB_TOKEN=ghp_your_token_here
   MIKA_GITHUB_REPO=owner/repo
   ```
3. Restart mika-server (tool registry is lazily initialized on first
   investigation request)

### Example: Override model in home config

In `~/.mika/config.toml`:

```toml
llm_model = "claude-opus-4-6"
log_level = "debug"
```

### Example: Override model via environment variable

```sh
export MIKA_LLM_MODEL=claude-haiku-4-5
```

The environment variable takes precedence over all config files and `.env`.

---

## CLI Config Management

The `mika config` command provides subcommands for viewing and modifying
configuration without editing files manually.

### `mika config` (no subcommand)

Prints a summary of current settings (model, max tokens, log level, auth status).

### `mika config get <key>`

Read a single configuration value:

```sh
mika config get llm_model          # prints: claude-sonnet-4-6
mika config get llm_model --verbose # prints: llm_model = claude-sonnet-4-6 (source: default, backend: File)
```

The `--verbose` flag shows where the value comes from (env var, agent config.toml,
global config.toml, .env file, database, or default).

### `mika config set <key> [value]`

Write a configuration value:

```sh
mika config set llm_model claude-opus-4-6     # writes to agent config.toml
mika config set llm_max_tokens 8192            # validated as integer
mika config set llm_api_key                 # secret: prompts interactively, writes to .env
```

Behavior depends on the key's backend:

| Backend | Behavior |
|---------|----------|
| `File` | Writes to `{agent_home}/config.toml` (preserves comments, atomic write). Validates value format. |
| `Env` | Writes to `~/.mika/.env`. Secret keys prompt interactively via masked input (never accepts CLI arguments). Non-secret keys accept a CLI value or prompt with visible input. |
| `Database` | Writes to the `customer_config` table in SQLite. |
| `ReadOnly` | Rejected with an error. |

### `mika config list`

Show all known configuration keys with their current values:

```sh
mika config list             # key-value pairs
mika config list --verbose   # includes source and backend per key
```

Secret values are displayed as `[REDACTED]`. Unset optional values show `[NOT SET]`.

### Other subcommands

- `mika config edit` -- Opens `identity.toml` in `$EDITOR`.
- `mika config soul` -- Prints `soul.md` to stdout.

---

## Settings Reference

Complete table of all `Settings` struct fields for the agent (CLI and server modes).

| Field | Type | Default | Env Var | Description |
|-------|------|---------|---------|-------------|
| `llm_api_key` | `Option<String>` | None | `MIKA_LLM_API_KEY` | LLM API key (Anthropic, OpenAI, Groq, etc.). Auto-detected from prefix (`sk-ant-oat` = OAuth, otherwise = API key). Required for any command that calls the Claude API. |
| `llm_model` | `String` | `claude-sonnet-4-6` | `MIKA_LLM_MODEL` | Claude model ID to use for inference. |
| `llm_max_tokens` | `u32` | `4096` | `MIKA_LLM_MAX_TOKENS` | Maximum tokens for Claude responses. |
| `db_path` | `PathBuf` | `~/.mika/data/mika.db` | `MIKA_DB_PATH` | Path to the SQLite database file. If not explicitly set, resolves to `{home_dir}/data/mika.db`. |
| `log_level` | `String` | `info` | `MIKA_LOG_LEVEL` | Log level filter. Valid values: `trace`, `debug`, `info`, `warn`, `error`. |
| `routing_url` | `Option<String>` | None | `MIKA_ROUTING_URL` | Gateway URL for outbound message delivery. Required in server mode. |
| `customer_id` | `Option<String>` | None | `MIKA_CUSTOMER_ID` | Customer identifier. Set per container in hosted deployments. |
| `server_port` | `u16` | `8080` | `MIKA_SERVER_PORT` | HTTP server listen port. Only used in server mode (`mika-server`). |
| `openai_api_key` | `Option<String>` | None | `MIKA_OPENAI_API_KEY` | OpenAI API key for embedding generation. Enables vector similarity in Layer 3 hybrid search. |
| `embedding_model` | `String` | `text-embedding-3-small` | `MIKA_EMBEDDING_MODEL` | OpenAI embedding model ID. |
| `embedding_dimensions` | `u32` | `512` | `MIKA_EMBEDDING_DIMENSIONS` | Embedding vector dimensions. |
| `brave_api_key` | `Option<String>` | None | `MIKA_BRAVE_API_KEY` | Brave Search API key for `web_search` builtin skill. Get a free key at https://brave.com/search/api/. |
| `investigate_github_token` | `Option<String>` | None | `MIKA_INVESTIGATE_GITHUB_TOKEN` | GitHub Personal Access Token for the investigation panel's issue creation tool. Needs `repo` scope for private repos or `public_repo` for public. Both `investigate_github_token` and `github_repo` must be set to enable the tool. |
| `github_repo` | `Option<String>` | None | `MIKA_GITHUB_REPO` | Target GitHub repository in `owner/repo` format (e.g. `senara-solutions/mika`). Validated at registration time — must contain exactly one `/`. |
| `internal_token` | `Option<SecretString>` | None | `MIKA_INTERNAL_TOKEN` | Shared bearer token for gateway-to-container auth. Must be exactly 64 hex characters (32 bytes hex-encoded). Required in server mode. Accepted on all routes (superuser). |
| `dashboard_token` | `Option<SecretString>` | None | `MIKA_DASHBOARD_TOKEN` | Separate bearer token for read-only dashboard API routes (`/api/v1/*`). If unset, dashboard routes accept `internal_token` (backwards compatible). Only grants access to read-only routes — mutation endpoints (`/message`, `/tasks/{id}/complete`) still require `internal_token`. |
| `server_log_file` | `Option<PathBuf>` | None | `MIKA_SERVER_LOG_FILE` | File path for mika-server log output. Logs go to stdout + file when set. |
| `disable_bundled_skills` | `bool` | `false` | `MIKA_DISABLE_BUNDLED_SKILLS` | Skip bundled skill re-sync on startup. Useful for debugging handler scripts. **Do not enable in production** — prevents security updates to handler scripts from propagating. |
| `telemetry_enabled` | `bool` | `false` | `MIKA_TELEMETRY_ENABLED` | Enable OpenTelemetry trace export. Requires `--features telemetry` at build time. When enabled, spans are exported via OTLP HTTP to the configured endpoint. |
| `otlp_endpoint` | `Option<String>` | None | `MIKA_OTLP_ENDPOINT` | OTLP endpoint URL for trace export — must include `/v1/traces` (e.g. `https://cloud.langfuse.com/api/public/otel/v1/traces` for Langfuse, `http://localhost:4318/v1/traces` for Jaeger). Required when `telemetry_enabled` is true. |
| `otlp_auth_header` | `Option<SecretString>` | None | `MIKA_OTLP_AUTH_HEADER` | OTLP authorization header value. For Langfuse, pass raw `publicKey:secretKey` (auto-encoded to Base64) or pre-encoded Base64. Sent as `Authorization: Basic <value>`. Zeroized on drop. |

The `home_dir` field is also present on the struct but is not configurable via
file or environment variable. It is resolved automatically from `$MIKA_HOME` or
defaults to `~/.mika/`.

### Security notes

- `llm_api_key`, `internal_token`, `dashboard_token`, `brave_api_key`, `investigate_github_token`, and `otlp_auth_header` are redacted in
  `Debug` output (printed as `[REDACTED]`). The `mika config` command
  distinguishes between credential types: `OAuth token [REDACTED]` or
  `API key [REDACTED]`.
- When `llm_api_key` contains an Anthropic credential, Mika detects the type
  from the `sk-ant-oat` prefix and adjusts the HTTP auth scheme automatically
  (Bearer + `anthropic-beta` header for OAuth, `x-api-key` header for standard
  keys). For non-Anthropic providers, the key is sent as a Bearer token.
- Secrets should be set in `~/.mika/.env` or via shell environment variables, never committed to config files.
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
heartbeat cycles. In server mode, heartbeats are triggered by an external
scheduler hitting the `/heartbeat` endpoint. The agent reads this file and uses it
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
| `MIKA_LLM_API_KEY` | Yes | LLM API key (Anthropic, OpenAI, Groq, etc.) |
| `MIKA_LLM_MODEL` | No | Override model (default: `claude-sonnet-4-6`) |
| `MIKA_LLM_MAX_TOKENS` | No | Override max tokens (default: `4096`) |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level (default: `info`) |
| `MIKA_HOME` | No | Override home directory (default: `~/.mika/`) |
| `MIKA_OPENAI_API_KEY` | No | OpenAI API key for Layer 3 vector search |
| `MIKA_BRAVE_API_KEY` | No | Brave Search API key for web search skill |
| `MIKA_INVESTIGATE_GITHUB_TOKEN` | No | GitHub token for investigation panel issue creation |
| `MIKA_GITHUB_REPO` | No | GitHub repo (`owner/repo`) for issue creation |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |
| `MIKA_TELEMETRY_ENABLED` | No | Enable OTel trace export (requires `--features telemetry` build) |
| `MIKA_OTLP_ENDPOINT` | No | OTLP endpoint URL with `/v1/traces` path (required when telemetry enabled) |
| `MIKA_OTLP_AUTH_HEADER` | No | OTLP auth header value (e.g. Base64-encoded Langfuse credentials) |

### Server mode

For running `mika-server` (the HTTP server in hosted mode), additional variables
are required for inter-service communication:

| Variable | Required | Description |
|----------|----------|-------------|
| `MIKA_LLM_API_KEY` | Yes | LLM API key (Anthropic, OpenAI, Groq, etc.) |
| `MIKA_ROUTING_URL` | Yes | Gateway URL for outbound message delivery |
| `MIKA_INTERNAL_TOKEN` | Yes | Shared bearer token (64 hex chars) for gateway auth |
| `MIKA_CUSTOMER_ID` | Yes | Customer identifier for this container |
| `MIKA_SERVER_PORT` | No | Listen port (default: `8080`) |
| `MIKA_LLM_MODEL` | No | Override model |
| `MIKA_LLM_MAX_TOKENS` | No | Override max tokens |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |
| `MIKA_TELEMETRY_ENABLED` | No | Enable OTel trace export (requires `--features telemetry` build) |
| `MIKA_OTLP_ENDPOINT` | No | OTLP endpoint URL with `/v1/traces` path (required when telemetry enabled) |
| `MIKA_OTLP_AUTH_HEADER` | No | OTLP auth header value (e.g. Base64-encoded Langfuse credentials) |
| `MIKA_CORS_ORIGIN` | No | Allowed origin for dashboard CORS (default: `http://localhost:5173`) |
| `MIKA_DASHBOARD_TOKEN` | No | Separate bearer token for read-only dashboard API routes (`/api/v1/*`). If unset, dashboard routes accept `MIKA_INTERNAL_TOKEN`. |

### Token Generation

`mika setup` automatically generates a compliant `MIKA_INTERNAL_TOKEN` and saves
it to `~/.mika/.env`. To generate a token manually:

```sh
openssl rand -hex 32
```

`MIKA_INTERNAL_TOKEN` is validated on startup: it must be exactly 64 hexadecimal
characters (32 bytes hex-encoded). Invalid values cause an immediate startup error.

### Gateway mode

For running `mika-gateway` (the Telegram webhook router), these additional
variables are required:

| Variable | Required | Description |
|----------|----------|-------------|
| `MIKA_DATABASE_URL` | Yes | Postgres connection string |
| `MIKA_TELEGRAM_BOT_TOKEN` | Yes | Telegram Bot API token |
| `MIKA_TELEGRAM_WEBHOOK_SECRET` | Yes | 64-char hex secret for webhook validation |
| `MIKA_TELEGRAM_WEBHOOK_URL` | Yes | Public HTTPS URL for Telegram webhook delivery |
| `MIKA_INTERNAL_TOKEN` | Yes | Shared bearer token (64 hex chars) for container auth |
| `MIKA_GATEWAY_PORT` | No | Listen port (default: `8080`) |
| `MIKA_LOG_LEVEL` | No | Log level (default: `info`) |
| `MIKA_AGENT_BASE_URL` | No | Override agent container URL for local E2E testing |
| `MIKA_GATEWAY_LOG_FILE` | No | Optional log file path |

Both `MIKA_INTERNAL_TOKEN` and `MIKA_TELEGRAM_WEBHOOK_SECRET` must be exactly 64
hexadecimal characters (32 bytes hex-encoded). Generate with `openssl rand -hex 32`.

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
llm_model = "claude-opus-4-6"
```

**Via environment variable:**

```sh
export MIKA_LLM_MODEL=claude-opus-4-6
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
