---
title: Configuration
description: Configuration files, environment variables, and config cascade
---

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

Settings are loaded from multiple sources, in order of increasing priority. A value
set at a higher layer overrides the same value from a lower layer.

| Priority | Source                    | Description                                |
|----------|---------------------------|--------------------------------------------|
| 1 (lowest) | Rust defaults           | Compiled-in serde defaults (e.g. `claude-sonnet-4-6`) |
| 2        | TOML config files         | `~/.mika/config.toml` + optional `~/.mika/agents/X/config.toml` |
| 3        | Per-agent `.env`          | `~/.mika/agents/X/.env` — parsed inline, not set in process env (server mode) |
| 4        | Global `.env`             | `~/.mika/.env` — loaded into process env via dotenvy |
| 5 (highest) | `MIKA_*` env vars      | Shell environment variables, always win    |

In **CLI mode** (single agent per process), the per-agent `.env` is loaded into the
process environment before the global `.env` (dotenvy first-write-wins). In **server
mode** (multiple agents per process), per-agent `.env` files are parsed without
mutating process env and injected as inline TOML config sources — each agent gets
its own secrets without cross-contamination.

All config files are optional. If a file does not exist, it is silently skipped.

### Secrets in `~/.mika/.env`

Store API keys and tokens in `~/.mika/.env` instead of config files or shell
profiles. This file is auto-loaded on startup and does **not** override
existing shell environment variables. File permissions are set to `0600`.

```sh
# ~/.mika/.env
MIKA_ANTHROPIC_API_KEY=sk-ant-api03-...
MIKA_OPENAI_API_KEY=sk-...        # Also used for Layer 3 vector search
MIKA_BRAVE_API_KEY=BSA...
MIKA_GITHUB_APP_ID=123456              # GitHub App (preferred over PAT)
MIKA_GITHUB_APP_PRIVATE_KEY=<base64>   # base64 -w0 < your-app.pem
MIKA_GITHUB_APP_INSTALLATION_ID=78901234
MIKA_GITHUB_APP_LOGIN=mika-dev[bot]    # Bot login for assignee filtering (optional)
MIKA_GITHUB_TOKEN=ghp_...              # Agent operations (PAT fallback)
MIKA_INVESTIGATE_GITHUB_TOKEN=ghp_...  # Investigation panel only (issue creation)
MIKA_GITHUB_REPO=owner/repo
# GH_TOKEN — do NOT set here; run_gh injects MIKA_GITHUB_TOKEN as GH_TOKEN automatically
```

Run `mika setup` to interactively configure secrets (API keys, tokens) and
preferences (telemetry) — secrets are written to `~/.mika/.env`, config to
`~/.mika/config.toml`. The wizard auto-generates `MIKA_INTERNAL_TOKEN` for
server mode.

### Per-agent secrets in `~/.mika/agents/<name>/.env`

In a multi-agent setup, each agent can have its own `.env` file for agent-specific
secrets (e.g., different GitHub App credentials per agent):

```sh
# ~/.mika/agents/mika-qa/.env
MIKA_GITHUB_APP_ID=654321
MIKA_GITHUB_APP_PRIVATE_KEY=<base64>
MIKA_GITHUB_APP_INSTALLATION_ID=98765432
MIKA_GITHUB_APP_LOGIN=mika-qa[bot]
```

Per-agent `.env` values override the global `~/.mika/.env` but not shell env vars.

### GitHub App authentication (preferred)

GitHub App installation tokens are preferred over Personal Access Tokens for agent
operations. They are short-lived (1 hour), org-scoped, and auditable.

1. Register a GitHub App on your organization (Settings → Developer settings → GitHub Apps)
2. Note the **App ID** and **Installation ID** (visible after installing on the org)
3. Download the private key PEM file and base64-encode it:
   ```sh
   base64 -w0 < your-app.pem
   ```
4. Add to `~/.mika/.env`:
   ```sh
   MIKA_GITHUB_APP_ID=123456
   MIKA_GITHUB_APP_PRIVATE_KEY=<paste base64 output here>
   MIKA_GITHUB_APP_INSTALLATION_ID=78901234
   MIKA_GITHUB_APP_LOGIN=mika-dev[bot]  # optional — bot login for assignee filtering
   ```

All 3 core env vars must be set (ID, key, installation ID). `MIKA_GITHUB_APP_LOGIN` is optional. The private key is validated at startup — base64 decode
and RSA PEM parse errors are reported immediately. When configured, Mika generates
RS256 JWTs and exchanges them for installation tokens with automatic caching
(5-minute pre-expiry refresh). If the token exchange fails, Mika falls back to
`MIKA_GITHUB_TOKEN` PAT with a warning.

Run `mika doctor` to verify the configuration.

### GitHub token for agent operations (PAT fallback)

`MIKA_GITHUB_TOKEN` enables agent-level GitHub operations: context injection (fetching
PR diffs), work item enrichment (PR/issue status), and dev-run PR merges. When a
GitHub App is configured (see above), the App's installation token takes precedence.

1. Create a GitHub Personal Access Token:
   - **Fine-grained token** (recommended): Settings → Developer settings →
     Fine-grained tokens → select your repo → Permissions → Pull requests: Read and Write,
     Issues: Read and Write, Contents: Read
2. Add to `~/.mika/.env`:
   ```sh
   MIKA_GITHUB_TOKEN=ghp_your_token_here
   ```

### GitHub token for `gh` CLI in Claude Code sessions

`GH_TOKEN` is used by the `gh` CLI in Claude Code sessions spawned via claude-pilot.
Without it, `gh` falls back to the host user's `~/.config/gh/hosts.yml` (personal account).

**Do NOT set `GH_TOKEN` in `~/.mika/.env`.** If detected there at startup,
`check_env_warnings()` actively removes it from the process environment and emits a warning.
Additionally, `scrub_mika_env_vars()` scrubs `GH_TOKEN` from exec handler child processes
via `EXTRA_SCRUB_VARS` as defense-in-depth. The `run_gh` builtin handler re-injects
`MIKA_GITHUB_TOKEN` as `GH_TOKEN` AFTER the scrub for correct platform identity separation.
The same scrub-then-inject pattern is applied to all exec handler subprocesses spawned by
skills — `MIKA_GITHUB_TOKEN` is re-injected as `GH_TOKEN` after the env scrub, so any `gh`
CLI invocation inside a skill handler runs as the agent's configured GitHub identity (not
the host user).

| Layer | Identity | Purpose |
|-------|----------|---------|
| Host `gh auth` / `GH_TOKEN` in shell | Developer account | Claude Code / claude-pilot: PR creation, git push |
| `MIKA_GITHUB_TOKEN` (injected as `GH_TOKEN` by `run_gh`) | Platform account | Agent operations: QA reviews, PR comments, issue management |

`GH_TOKEN` should only be set at the host level (e.g., shell profile, CI environment) for
Claude Code sessions that need GitHub access via claude-pilot.

### GitHub issue creation (dashboard investigation)

The investigation panel can create GitHub issues when both `MIKA_INVESTIGATE_GITHUB_TOKEN`
and `MIKA_GITHUB_REPO` are set. This token is separate from `MIKA_GITHUB_TOKEN` — the
investigation panel uses only `MIKA_INVESTIGATE_GITHUB_TOKEN`. Steps:

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
llm_provider = "anthropic"
anthropic_model = "claude-opus-4-6"
log_level = "debug"
```

### Example: Override provider via environment variable

```sh
export MIKA_LLM_PROVIDER=openai
export MIKA_OPENAI_MODEL=gpt-4-turbo
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
mika config get llm_provider            # prints: anthropic
mika config get llm_provider --verbose  # prints: llm_provider = anthropic (source: default, backend: File)
```

The `--verbose` flag shows where the value comes from (env var, agent config.toml,
global config.toml, .env file, database, or default).

### `mika config set <key> [value]`

Write a configuration value:

```sh
mika config set llm_provider openai               # writes to agent config.toml
mika config set anthropic_model claude-opus-4-6    # writes to agent config.toml
mika config set llm_max_tokens 8192                # validated as integer
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
| `llm_provider` | `ProviderKind` | `anthropic` | `MIKA_LLM_PROVIDER` | Active LLM provider. One of: `anthropic`, `openai`, `openrouter`, `groq`, `ollama`, `mistral`, `google`, `deepseek`. Each provider has per-provider `{prefix}_model`, `{prefix}_api_key`, `{prefix}_base_url` fields. See [LLM Provider Configuration](#llm-provider-configuration). |
| `{provider}_model` | `Option<String>` | Provider default | `MIKA_{PROVIDER}_MODEL` | Model ID for the provider. Falls back to provider's default model if not set. |
| `{provider}_api_key` | `Option<String>` | None | `MIKA_{PROVIDER}_API_KEY` | API key for the provider. Stored in `.env`. `MIKA_OPENAI_API_KEY` is shared with embeddings. |
| `{provider}_base_url` | `Option<String>` | Provider default | `MIKA_{PROVIDER}_BASE_URL` | Override base URL for the provider. Each provider has a built-in default. |
| `llm_max_tokens` | `u32` | `4096` | `MIKA_LLM_MAX_TOKENS` | Maximum tokens for LLM responses. |
| `db_path` | `PathBuf` | `~/.mika/data/mika.db` | `MIKA_DB_PATH` | Path to the SQLite database file. If not explicitly set, resolves to `{home_dir}/data/mika.db`. |
| `log_level` | `String` | `info` | `MIKA_LOG_LEVEL` | Log level filter. Valid values: `trace`, `debug`, `info`, `warn`, `error`. |
| `log_format` | `String` | `json` | `MIKA_LOG_FORMAT` | Stdout log format for mika-server and mika-gateway: `json` (default) or `pretty` (human-readable). CLI always uses pretty format regardless of this setting. File output always uses JSON. |
| `routing_url` | `Option<String>` | None | `MIKA_ROUTING_URL` | Gateway URL for outbound message delivery. Required in server mode. |
| `customer_id` | `Option<String>` | None | `MIKA_CUSTOMER_ID` | Customer identifier. Set per container in hosted deployments. |
| `server_port` | `u16` | `8080` | `MIKA_SERVER_PORT` | HTTP server listen port. Only used in server mode (`mika-server`). |
| `openai_api_key` | `Option<String>` | None | `MIKA_OPENAI_API_KEY` | OpenAI API key for embedding generation. Enables vector similarity in Layer 3 hybrid search. |
| `embedding_model` | `String` | `text-embedding-3-small` | `MIKA_EMBEDDING_MODEL` | OpenAI embedding model ID. |
| `embedding_dimensions` | `u32` | `512` | `MIKA_EMBEDDING_DIMENSIONS` | Embedding vector dimensions. |
| `brave_api_key` | `Option<String>` | None | `MIKA_BRAVE_API_KEY` | Brave Search API key for `web_search` builtin skill. Get a free key at https://brave.com/search/api/. |
| `kg_ingestion_model` | `Option<String>` | None | `MIKA_KG_INGESTION_MODEL` | Shared fallback model for KG extraction and resolution. Format: `provider/model` (e.g., `anthropic/claude-haiku-4-5-20251001`). If unset, KG features requiring LLM calls are disabled. |
| `kg_extraction_model` | `Option<String>` | None | `MIKA_KG_EXTRACTION_MODEL` | Model for NER + fact-triple extraction (#690). Falls back to `kg_ingestion_model` if unset. Cheap/fast tier recommended. |
| `github_token` | `Option<String>` | None | `MIKA_GITHUB_TOKEN` | GitHub Personal Access Token for agent operations (context injection, work item enrichment, PR merge). Needs Pull requests R/W, Issues R/W, Contents R scopes. |
| `investigate_github_token` | `Option<String>` | None | `MIKA_INVESTIGATE_GITHUB_TOKEN` | GitHub Personal Access Token for the investigation panel's issue creation tool only. Needs `repo` scope for private repos or `public_repo` for public. Both `investigate_github_token` and `github_repo` must be set to enable the tool. |
| `github_repo` | `Option<String>` | None | `MIKA_GITHUB_REPO` | Target GitHub repository in `owner/repo` format (e.g. `senara-solutions/mika`). Validated at registration time — must contain exactly one `/`. |
| `internal_token` | `Option<SecretString>` | None | `MIKA_INTERNAL_TOKEN` | Shared bearer token for gateway-to-container auth. Must be exactly 64 hex characters (32 bytes hex-encoded). Required in server mode. Accepted on all routes (superuser). |
| `dashboard_token` | `Option<SecretString>` | None | `MIKA_DASHBOARD_TOKEN` | Separate bearer token for read-only dashboard API routes (`/api/v1/*`). If unset, dashboard routes accept `internal_token` (backwards compatible). Only grants access to read-only routes — mutation endpoints (`/message`, `/tasks/{id}/complete`) still require `internal_token`. |
| `server_log_file` | `Option<PathBuf>` | None | `MIKA_SERVER_LOG_FILE` | File path for mika-server log output. Logs go to stdout + file when set. |
| `dashboard_enabled` | `bool` | `false` | `MIKA_DASHBOARD_ENABLED` | Enable embedded dashboard SPA at `/dashboard/`. When enabled, the pre-built React dashboard is served from the binary via `rust-embed`. Requires `MIKA_DASHBOARD_TOKEN` for token injection. Build the dashboard before compiling: `npm run build --prefix dashboard` (`VITE_BASE_PATH` is set automatically). |
| `disable_bundled_skills` | `bool` | `false` | `MIKA_DISABLE_BUNDLED_SKILLS` | Skip bundled skill re-sync on startup. Useful for debugging handler scripts. **Do not enable in production** — prevents security updates to handler scripts from propagating. |
| `dev_mode` | `bool` | `false` | `MIKA_DEV_MODE` | Enable dev mode — auto-provisions well-known development agents (`mika-dev`, `mika-qa`) on startup with role-specific identity, soul, and skill assignments. Idempotent — existing agents are never overwritten. |
| `disable_agent_provisioning` | `bool` | `false` | `MIKA_DISABLE_AGENT_PROVISIONING` | Skip well-known agent auto-creation on startup. When true, prevents `dev_mode` from creating or updating agent identity files, allowing manual edits to persist across restarts/deploys. |
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
- When `anthropic_api_key` contains an Anthropic credential, Mika detects the
  type from the `sk-ant-oat` prefix and adjusts the HTTP auth scheme automatically
  (Bearer + `anthropic-beta` header for OAuth, `x-api-key` header for standard
  keys). For non-Anthropic providers, the key is sent as a Bearer token.
- **OAuth PKCE flow:** When `MIKA_ANTHROPIC_API_KEY` starts with `sk-ant-oat`,
  Mika uses an `OAuthTokenManager` that transparently exchanges the subscription
  token for a short-lived access token via PKCE. Tokens are cached in
  `~/.mika/oauth.json` (0600 permissions) and auto-refreshed 60 seconds before
  expiry. Initial setup requires `mika setup --mode oauth` (interactive
  browser-based authorization). A SHA-256 hash of the subscription token is
  stored to detect token changes.
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
| `MIKA_LLM_PROVIDER` | No | Active LLM provider (default: `anthropic`) |
| `MIKA_{PROVIDER}_API_KEY` | Yes* | API key for the active provider |
| `MIKA_{PROVIDER}_MODEL` | No | Override model for a provider |
| `MIKA_{PROVIDER}_BASE_URL` | No | Override base URL for a provider |
| `MIKA_LLM_MAX_TOKENS` | No | Override max tokens (default: `4096`) |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level (default: `info`) |
| `MIKA_HOME` | No | Override home directory (default: `~/.mika/`) |
| `MIKA_OPENAI_API_KEY` | No | OpenAI API key (LLM + Layer 3 vector search) |
| `MIKA_BRAVE_API_KEY` | No | Brave Search API key for web search skill |
| `MIKA_KG_INGESTION_MODEL` | No | Shared fallback KG model (`provider/model` format) |
| `MIKA_KG_EXTRACTION_MODEL` | No | KG extraction model (falls back to `MIKA_KG_INGESTION_MODEL`) |
| `MIKA_GITHUB_TOKEN` | No | GitHub token for agent operations |
| `MIKA_INVESTIGATE_GITHUB_TOKEN` | No | GitHub token for investigation panel issue creation only |
| `MIKA_GITHUB_REPO` | No | GitHub repo (`owner/repo`) for issue creation |
| `MIKA_DEV_MODE` | No | Auto-provision mika-dev + mika-qa agents on startup (default: false) |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |
| `MIKA_DISABLE_AGENT_PROVISIONING` | No | Prevent dev_mode from overwriting agent files (default: false) |

\* Set the API key for the active provider. E.g., `MIKA_ANTHROPIC_API_KEY` for Anthropic, `MIKA_GROQ_API_KEY` for Groq. Ollama does not require an API key.
| `MIKA_SERVER_URL` | No | mika-server URL for dashboard CLI commands (default: `http://localhost:8080`) |
| `MIKA_GATEWAY_URL` | No | mika-gateway URL for webhook DLQ CLI commands (default: `http://localhost:3001`) |
| `MIKA_TELEMETRY_ENABLED` | No | Enable OTel trace export (requires `--features telemetry` build) |
| `MIKA_OTLP_ENDPOINT` | No | OTLP endpoint URL with `/v1/traces` path (required when telemetry enabled) |
| `MIKA_OTLP_AUTH_HEADER` | No | OTLP auth header value (e.g. Base64-encoded Langfuse credentials) |

### Server mode

For running `mika-server` (the HTTP server in hosted mode), additional variables
are required for inter-service communication:

| Variable | Required | Description |
|----------|----------|-------------|
| `MIKA_{PROVIDER}_API_KEY` | Yes | API key for the active LLM provider |
| `MIKA_ROUTING_URL` | Yes | Gateway URL for outbound message delivery |
| `MIKA_INTERNAL_TOKEN` | Yes | Shared bearer token (64 hex chars) for gateway auth |
| `MIKA_CUSTOMER_ID` | Yes | Customer identifier for this container |
| `MIKA_SERVER_PORT` | No | Listen port (default: `8080`) |
| `MIKA_LLM_PROVIDER` | No | Active LLM provider (default: `anthropic`) |
| `MIKA_LLM_MAX_TOKENS` | No | Override max tokens |
| `MIKA_DB_PATH` | No | Override database path |
| `MIKA_LOG_LEVEL` | No | Override log level |
| `MIKA_LOG_FORMAT` | No | Stdout log format: `json` (default) or `pretty` |
| `MIKA_DEV_MODE` | No | Auto-provision mika-dev + mika-qa agents on startup (default: false) |
| `MIKA_DISABLE_BUNDLED_SKILLS` | No | Skip bundled skill re-sync on startup (default: false) |
| `MIKA_DISABLE_AGENT_PROVISIONING` | No | Prevent dev_mode from overwriting agent files (default: false) |
| `MIKA_TELEMETRY_ENABLED` | No | Enable OTel trace export (requires `--features telemetry` build) |
| `MIKA_OTLP_ENDPOINT` | No | OTLP endpoint URL with `/v1/traces` path (required when telemetry enabled) |
| `MIKA_OTLP_AUTH_HEADER` | No | OTLP auth header value (e.g. Base64-encoded Langfuse credentials) |
| `MIKA_DASHBOARD_ENABLED` | No | Enable embedded dashboard SPA at `/dashboard/` (default: `false`) |
| `MIKA_CORS_ORIGIN` | No | Allowed origin for dashboard CORS (default: `http://localhost:5173`) |
| `MIKA_DASHBOARD_TOKEN` | No | Separate bearer token for read-only dashboard API routes (`/api/v1/*`). Required for embedded dashboard token injection. If unset, dashboard API routes accept `MIKA_INTERNAL_TOKEN`. |

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
| `MIKA_LOG_FORMAT` | No | Stdout log format: `json` (default) or `pretty` |
| `MIKA_AGENT_BASE_URL` | No | Override agent container URL for local E2E testing |
| `MIKA_AGENTS_NAMESPACE` | No | K8s namespace where agent pods run (default: `mika-agents`). Used for FQDN construction in cross-namespace DNS resolution. |
| `MIKA_GATEWAY_LOG_FILE` | No | Optional log file path |

Both `MIKA_INTERNAL_TOKEN` and `MIKA_TELEGRAM_WEBHOOK_SECRET` must be exactly 64
hexadecimal characters (32 bytes hex-encoded). Generate with `openssl rand -hex 32`.

---

## LLM Provider Configuration

Mika supports 8 LLM providers via the `LlmProvider` trait. Each provider has its
own `model`, `api_key`, and `base_url` fields. The active provider is selected by
`llm_provider` in `config.toml`.

### Supported providers

| Provider | Config value | Default Model | Default Base URL | API Key Env Var |
|----------|-------------|---------------|------------------|-----------------|
| Anthropic (default) | `anthropic` | `claude-sonnet-4-6` | `https://api.anthropic.com` | `MIKA_ANTHROPIC_API_KEY` |
| OpenAI | `openai` | `gpt-4o` | `https://api.openai.com/v1` | `MIKA_OPENAI_API_KEY` |
| OpenRouter | `openrouter` | `anthropic/claude-sonnet-4` | `https://openrouter.ai/api/v1` | `MIKA_OPENROUTER_API_KEY` |
| Groq | `groq` | `llama-3.3-70b-versatile` | `https://api.groq.com/openai/v1` | `MIKA_GROQ_API_KEY` |
| Ollama | `ollama` | `llama3` | `http://localhost:11434/v1` | `MIKA_OLLAMA_API_KEY` (optional) |
| Mistral | `mistral` | `mistral-large-latest` | `https://api.mistral.ai/v1` | `MIKA_MISTRAL_API_KEY` |
| Google AI | `google` | `gemini-2.5-flash` | `https://generativelanguage.googleapis.com/v1beta/openai` | `MIKA_GOOGLE_API_KEY` |
| DeepSeek | `deepseek` | `deepseek-chat` | `https://api.deepseek.com` | `MIKA_DEEPSEEK_API_KEY` |

### Per-provider configuration

Each provider has three config keys with a `{provider}_` prefix:

| Key pattern | Example | Description |
|-------------|---------|-------------|
| `{provider}_model` | `anthropic_model = "claude-opus-4-6"` | Override the default model |
| `{provider}_api_key` | Set via `MIKA_ANTHROPIC_API_KEY` env var | API key (stored in `.env`) |
| `{provider}_base_url` | `openai_base_url = "http://custom:8000/v1"` | Override the default base URL |

### Switching providers

**config.toml** (persisted):

```toml
# ~/.mika/config.toml
llm_provider = "anthropic"
anthropic_model = "claude-opus-4-6"
```

**Environment variables** (override config.toml):

```sh
export MIKA_LLM_PROVIDER=openai
export MIKA_OPENAI_API_KEY=sk-...
# Model defaults to gpt-4o, or override:
export MIKA_OPENAI_MODEL=gpt-4-turbo
```

**Ollama (local, no key needed):**

```sh
export MIKA_LLM_PROVIDER=ollama
# Model defaults to llama3, base URL defaults to localhost:11434
```

**Groq:**

```sh
export MIKA_LLM_PROVIDER=groq
export MIKA_GROQ_API_KEY=gsk_...
```

**Google Gemini:**

```sh
export MIKA_LLM_PROVIDER=google
export MIKA_GOOGLE_API_KEY=...
export MIKA_GOOGLE_MODEL=gemini-2.5-pro
```

### Runtime switching

In the TUI chat, use slash commands to switch providers and models at runtime:

```
/provider openai         # Switch to OpenAI (uses default model)
/provider set model gpt-4-turbo  # Override model for current provider
/model sonnet            # Switch model (aliases: sonnet, opus, haiku, gpt4o, deepseek, gemini)
```

Changes via `/provider` are persisted to `config.toml`. Changes via `/model` are
persisted to the provider-specific model key in `config.toml`.

### Provider capabilities

Not all providers support all features. The `LlmProvider` trait reports capabilities:

| Feature | Anthropic | OpenAI | OpenRouter | Groq | Ollama | Mistral | Google | DeepSeek |
|---------|-----------|--------|------------|------|--------|---------|--------|----------|
| Tool calling | Yes | Yes | Yes | Yes | Varies | Yes | Yes | Yes |
| Vision/images | Yes | Yes | Yes | No | No | Yes | Yes | Yes |
| Extended thinking | Yes | No | No | No | No | No | No | No |

When using a provider that doesn't support tool calling, Mika's agent tools
(memory, reminders, etc.) will not be available. The agent will operate in
text-only mode.

### Migration from v0.x

If you're upgrading from a version that used `llm_model`, `llm_api_key`, and
`llm_base_url`, update your configuration:

| Old | New |
|-----|-----|
| `llm_model = "claude-sonnet-4-6"` | `llm_provider = "anthropic"` (model defaults to claude-sonnet-4-6) |
| `llm_model = "openai/gpt-4o"` | `llm_provider = "openai"` + `openai_model = "gpt-4o"` |
| `MIKA_LLM_API_KEY=sk-ant-...` | `MIKA_ANTHROPIC_API_KEY=sk-ant-...` |
| `MIKA_LLM_API_KEY=sk-...` (OpenAI) | `MIKA_OPENAI_API_KEY=sk-...` |
| `MIKA_LLM_BASE_URL=...` | `{provider}_base_url = "..."` in config.toml |

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
