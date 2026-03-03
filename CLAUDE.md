# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation. Each customer gets their own agent container with SQLite storage. A shared gateway (`mika-gateway`) routes Telegram messages to the correct container.

**Current phase:** Phase 4 — Deployment infrastructure (Dockerfiles done, CI/CD done).

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → Claude API → match stop_reason → execute tools or respond
- **LLM:** Claude (Sonnet 4.6 default) via direct reqwest calls to Messages API
- **Database:** SQLite via rusqlite (per-customer)
- **HTTP server:** Axum 0.8 (mika-server binary) with tower-http middleware
- **HTTP client:** reqwest 0.12 with rustls-tls (Claude API client with typed errors and retry)
- **Async runtime:** tokio
- **MCP client:** rmcp 0.17 (official Rust MCP SDK) — stdio and Streamable HTTP transports
- **Config:** config-rs with `MIKA_` env prefix
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev)

## Directory Structure

- `crates/mika-common/` — Shared library: config, Claude API client, logging, home directory
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, HTTP server binary
- `crates/mika-gateway/` — Telegram webhook router: Postgres customer registry, message routing, pairing flow, outbound relay
- `crates/mika-cli/` — TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (status, memory, reminders, config, setup, mcp, skills). Global flags: `--agent <name>` (override active agent), `--team <name>` (launch TUI in team mode, mutually exclusive with `--agent`). Team mode sends user messages as goals to `run_team()`, streams typed `TeamEvent` callbacks (progress, agent messages, errors) as system messages, shows deliverables as assistant responses; `/verbose` toggles display of individual agent responses in team mode; team runs and message graphs persisted to per-team SQLite DB (`{team_dir}/data/mika.db`); agent-specific slash commands are gated via an allowlist. TUI slash commands: `/think` (persistent thinking level), `/model` (runtime model switching), `/agent` (agent switching). Shell-like Tab completion: bash-style longest-common-prefix for command names, context-aware argument completers (model aliases, thinking levels, agent/team/skill names, config keys+values, file paths with tilde expansion). `CompletionMode` state machine (Hidden/Command/Argument) with contextual popup titles. Smart Enter (execute argless, transition for arg commands). Supports image paste via Ctrl+V (arboard + xclip/wl-paste fallbacks on Linux). Persistent input history (`{home_dir}/.input_history` JSON, per-agent, atomic writes, 0600 permissions). Shell-like Up/Down arrows (cursor-position-aware, draft saving). Bracketed paste inserts at cursor position with `\r\n` normalization and 100KB size limit. Mouse scroll and Ctrl+Up/Down for conversation scrolling. Unicode-width-aware input text wrapping. Scroll and new-message indicators in footer.
- `config/` — Configuration files (default.toml; local.toml is gitignored)
- `docs/` — Public documentation (architecture, configuration, deployment, skills, slash-commands, getting-started)
- `docs/adr/` — Architecture Decision Records (numbered)
- `docs/openapi/` — OpenAPI specs (mika-server.yaml, gateway.yaml)
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands (`/mika` — full dev workflow, `/mika-doc-audit` — standalone documentation audit)

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Database:** Case-insensitive COLLATE NOCASE on unique text columns.
- **Secrets:** `Settings` has manual `Debug` impl that redacts API key. API key errors are opaque. Exec handler executor scrubs all MIKA_* env vars from child processes (defense-in-depth). Shell-exec and github handlers additionally `unset` specific vars in their scripts. MCP child processes use `env_clear()` + allowlist. Git subprocesses scrub MIKA_* vars and set `GIT_TERMINAL_PROMPT=0`.
- **Tools:** Each tool validates inputs (empty check + 10,000 char max). `ToolContext` contains `{ db, session_id, home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client, brave_api_key }`. Tool trait uses `#[async_trait]` (Send futures, required for `tokio::spawn` in server handlers). Per-tool timeout override via `timeout_secs()` default method (returns `None` → uses 30s default). Shared `validate_and_resolve_path(path, base_dir)` helper in `tools/mod.rs` for path security (empty check, length limit, absolute rejection, traversal inspection, symlink check, canonicalize containment) — used by `write_file`, `write_workspace`, `read_workspace`. `write_file` tool writes files to the agent's home directory with overwrite confirmation flow: if the target exists, the current content is returned and the agent must re-call with `confirm: true`. All three file tools (`write_file`, `write_workspace`, `read_workspace`) report resolved absolute paths in success/error messages so the agent can verify the actual file location. `PromptContext.home_dir` surfaces the agent's home directory absolute path in the system prompt so the agent knows write_file's base path.
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `mpsc` channel (closure-based dispatch). Clone-able, Send+Sync. Integrated into agent loop, tools, and scheduler.

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~834 tests)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, etc.)
- `cargo run --bin mika-server` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image

## Architecture

- **One container per customer**
- **Three-layer memory model:**
  - Layer 1: Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2000 token limit)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
  - Layer 3: Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid → FTS5-only → LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.
- **Agent loop:** Max 10 tool steps, 5-minute total timeout, 30s default per-tool timeout (overridable via `Tool::timeout_secs()`). Step-awareness nudge injected at step 8 (conversation mode only) to encourage wrapping up. On max-steps exceeded: continuation turn (tools disabled, 60s timeout) forces a text summary; if that fails, structured fallback shows last 5 tool names with status. Tool call summaries (name, truncated input/output, success) persisted in `conversations.metadata` JSON column for cross-turn introspection. History builder appends `<context type="tool_history">` blocks to assistant messages. Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.
- **Exec handler image protocol (`__mika_v1`):** Exec handler scripts can return images by outputting a JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. The executor detects the sentinel key via prefix check, reads and validates image files (canonicalize, regular file check, 5MB limit, magic-byte validation for JPEG/PNG/GIF/WebP), base64-encodes them, and returns them as `ImageData` on `ToolOutput`. File I/O runs in `tokio::spawn_blocking`. Max 5 images per result. The file-reader skill uses this protocol to return image files for visual analysis.
- **MCP (Model Context Protocol) client:** Connects to external MCP servers at startup via `McpManager`. Configured in `{agent_home}/mcp.json` (Claude Desktop convention). Supports stdio (child process) and Streamable HTTP transports via `rmcp` crate. HTTP transport supports custom `headers` (including `Authorization`) for authenticated servers; `Authorization` routed through rmcp's `auth_header()` (case-insensitive), other headers via `custom_headers()`. Tools namespaced as `mcp__{server}__{tool}` to prevent collisions. Dispatch chain: builtins → skills → MCP → unknown error. MCP tools excluded from silent/heartbeat mode. Child processes use `env_clear()` + allowlist (PATH, HOME, USER, LANG, TERM, TMPDIR, XDG_RUNTIME_DIR) + server-specific env, blocking MIKA_* overrides. Parallel connection via `JoinSet` for fast startup. Image limits: 5 per result, 5MB per image. Graceful degradation when no MCP servers configured. MCP available in CLI ask mode (per-invocation connections with graceful shutdown), CLI chat mode (session-persistent connections), and server mode. Team agents do not yet support MCP (`mcp_manager: None`). `mcp.json` written with `0600` permissions (may contain secrets). Manual `Debug` impl on `McpServerConfig` redacts both header and env values. CLI: `mika mcp add/remove/list/enable/disable`, `--header KEY=VALUE` for HTTP headers.
- **Management tools:** 6 tools for multi-agent/team workflows (`list_agents`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history`). Registered conditionally via `management_tools_if_needed()` only when `agents.len() > 1 || !teams.is_empty()`. Delegated agents receive `default_tools()` only (no management tools) to prevent recursion. `delegate_task` uses `run_team_agent()` with explicit `AsyncDatabase` shutdown for thread cleanup. Per-tool timeouts: `run_team` (300s), `delegate_task` (120s). `AgentParams` carries `global_home_dir: Option<&Path>` (e.g. `~/.mika/`) distinct from per-agent `home_dir` (e.g. `~/.mika/agents/main/`). `get_team_status` and `get_team_history` read from per-team SQLite DB (`{team_dir}/data/mika.db`), including message graph with parent_id links. System prompt shows "Agents & Teams" section with agent identities and team listings when management tools are active.
- **Skills marketplace:** Git-based skill distribution via `mika skills install/uninstall/update`. Clones repos to temp staging, scans for `skill.toml` (depth <= 2), copies skill dirs (no `.git/`, symlink escape checks), tracks in `marketplace.lock` (TOML, agent home root). GitHub shorthand supported (`user/repo`). Multi-skill repos show interactive picker (`dialoguer`). Three-tier origin: `[built-in]`, `[marketplace]`, `[custom]`. `delete_skill` tool cleans up lock entries. MIKA_* env vars scrubbed from git subprocesses. ADR-006.
- **Typed Claude API errors:** `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- **Audit log:** `memory_events` table tracks all memory mutations per session
- **Conversation compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt (not message history). Runs inline post-turn in CLI.
- **Silent mode agent loop:** Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`. Heartbeat mode uses `safe_always_on_skills()` which filters out exec/http-handler skills (e.g., tmux, shell-exec) for security — only builtin-handler skills are available in autonomous background runs. Silent prompt conditionally includes `send_message` guidance only when a message sender is configured.
- **MessageSender trait:** `#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. Text-only outbound: `send(&self, text: &str)` — tool-produced images are consumed by the LLM for visual analysis (via `tool_result` content blocks) and are never forwarded to the end user. CLI prints to stdout. Server uses `GatewayMessageSender` (POST to gateway `/send` with retry + failed_sends fallback).
- **Reminder scheduler:** `ReminderScheduler` uses owned types (no lifetime params). `recover()` fires past-due reminders on startup via silent agent. `spawn_poller()` runs a 60s background poll (`tokio::spawn`) that fires due reminders via `run_silent_agent`. Poller caps at 10 per cycle (`MAX_POLL_REMINDERS`), uses `MissedTickBehavior::Skip`, skips first tick (recovery already handled). In server mode, acquires `agent_lock` via `try_lock` before firing (defers if busy). CLI aborts poller on exit/agent-switch. Reminders stored as Unix timestamps (`fire_at INTEGER`) — migration v9 converted from TEXT (ISO 8601) to fix format mismatch where `datetime('now')` (space separator) vs stored `T` separator caused `fire_at <= datetime('now')` to always be false.
- **HTTP server (mika-server):** Axum-based with 3 endpoints: `/health` (no auth, health probes), `/message` (Bearer auth, 202 async, 10MB body limit, accepts optional base64 images), `/heartbeat` (Bearer auth, scheduled job trigger). `AppState` is Clone via Arc-wrapped deps. Agent lock (`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock` (429 if busy).
- **Heartbeat pre-filter:** Active hours (8-21 local via chrono-tz), rate limits (1/hour, 3/day), skip if user messaged within 2h. All checks before acquiring Mutex.
- **Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.
- **Schema version:** 11 (v7 adds: skills system tables; v8 adds: search_content, fts_search FTS5, vec_search vec0 for Layer 3; v9 converts: reminders.fire_at from TEXT to INTEGER unix timestamp; v10 adds: reflection_runs table for periodic memory reflection; v11 adds: team_runs and team_messages tables for graph-structured team persistence)
- **mika-gateway** (`crates/mika-gateway/` in this repo)**:** Telegram webhook router with Postgres customer registry. Handles text messages and images. Endpoints: `/webhook/telegram` (inbound), `/send` (outbound relay), `/health` + `/readyz` + `/livez` (health probes). Stateless, env-var-only config.
- **Docker images:** Multi-stage builds with dependency layer caching. Builder: `rust:1.93-slim`. `Dockerfile.agent` (95MB) for per-customer containers (runtime deps: ca-certificates, wget, file, jq, gh). `Dockerfile.gateway` for the stateless gateway (leaner: ca-certificates + wget only, no home dir). Both use rustls (no OpenSSL build deps). Both run as non-root user `mika` (UID 1000). Release profile: LTO + strip. **Host dependency:** `jq` is required by all skill handler scripts (shell-exec, tmux, github, file-reader) for JSON input parsing; handlers fail with a clear error if `jq` is not found.
- **CI/CD:** Three GitHub Actions workflows: `ci.yml` (PR checks: fmt, clippy, test), `release-plz.yml` (automated versioning, changelog, crates.io publishing, git tagging via conventional commits), `release.yml` (cross-platform binary builds on tag push: x86_64/aarch64 Linux + macOS). All actions pinned to commit SHAs. Binaries published to GitHub Releases with SHA256 checksums. Installer script: `install.sh`.

## Environment Variables

See `.env.example` for the full list. Required:
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key or OAuth subscription token. Auto-detected from prefix: `sk-ant-oat*` → OAuth bearer auth, otherwise → standard API key auth.

Optional (Layer 3 vector search):
- `MIKA_OPENAI_API_KEY` — OpenAI API key for embedding generation (enables vector similarity in hybrid search)

Optional (web search):
- `MIKA_BRAVE_API_KEY` — Brave Search API key for `web_search` builtin skill (get free key at https://brave.com/search/api/)

Server mode additionally requires:
- `MIKA_ROUTING_URL` — Gateway URL for outbound message delivery
- `MIKA_INTERNAL_TOKEN` — Shared secret for Bearer auth between gateway and agent

Optional (startup behavior):
- `MIKA_DISABLE_BUNDLED_SKILLS` — Skip bundled skill re-sync on startup (default: false). WARNING: do not enable in production — prevents security updates to handler scripts.

Optional (log files — logs go to stdout + file when set):
- `MIKA_SERVER_LOG_FILE` — File path for mika-server log output

Gateway mode (`mika-gateway` binary) additionally requires:
- `MIKA_DATABASE_URL` — Postgres connection string
- `MIKA_TELEGRAM_BOT_TOKEN` — Telegram Bot API token
- `MIKA_TELEGRAM_WEBHOOK_SECRET` — 64-char hex secret for webhook validation
- `MIKA_TELEGRAM_WEBHOOK_URL` — Public HTTPS URL for Telegram webhook delivery
- `MIKA_INTERNAL_TOKEN` — Shared 64-char hex bearer token (same as server mode)

## Pending Work

- **Deployment:** Deployment manifests, production deployment guide, Docker image CI
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `../openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `../lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
