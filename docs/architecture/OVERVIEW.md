# Mika Architecture Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation on Kubernetes.

## System Design

- **One container per customer** on Kubernetes for full data isolation
- Each customer gets their own agent container with plaintext SQLite storage on K8s encrypted volumes
- A shared gateway (mika-gateway) handles inbound Telegram/WhatsApp webhooks and routes messages to the correct container
- Agent containers communicate outbound via the gateway's `/send` endpoint

## Three-Layer Memory Model

### Layer 1: Core Memory
- Always included in the system prompt (~500 tokens per block)
- Agent-editable via `update_core_memory` tool
- Sections: user_summary, persona, goals, relationships, work_context
- 2000 token total limit across all sections

### Layer 2: Structured Facts
- Categories: People, Commitments, Preferences, Events
- Managed via `store_fact`, `update_fact`, `search_memory` tools
- Stored in SQLite with plaintext for portability

### Layer 3: Hybrid Search
- FTS5 full-text search + sqlite-vec cosine similarity
- Reciprocal Rank Fusion combines results from both engines
- Optional OpenAI embeddings (text-embedding-3-small, 512 dimensions)
- Graceful degradation: hybrid → FTS5-only → LIKE fallback
- Indexed on store_fact/update_fact, backfilled on startup

## Agent Loop

- Uses Claude API directly via reqwest (no agent framework)
- Flow: retrieve context → build system prompt → call Claude Messages API → match stop_reason → execute tools or respond
- Maximum 10 tool steps per turn, 5-minute total timeout, 30-second per-tool timeout
- Tool dispatch: builtin tools first, then skill-defined tools (exec/http/builtin handlers)
- Conversation compaction at 50 messages (keeps 20 recent, summarizes older via Claude)

## Skills System

- Skills are directories under `{agent_home}/skills/{skill_name}/`
- Each skill has: `skill.toml` (manifest), optional `system_prompt.md`, optional `tools.json`
- Trigger types: keyword matching (substring on user message) or `always_on = true`
- Tool handler types: `exec` (subprocess), `http` (HTTP call), `builtin` (Rust function)
- Bundled skills ship with the binary and are seeded on first run
- Skills can be enabled/disabled via `.disabled` marker file or `toggle_skill` tool

## HTTP Server (mika-server)

The per-customer agent container runs an Axum HTTP server with these endpoints:

- `GET /health` — K8s liveness/readiness probe (no auth)
- `POST /message` — Receives messages from gateway (Bearer auth, 202 async processing)
- `POST /heartbeat` — CronJob trigger for proactive check-ins (Bearer auth)

Architecture details:
- `AppState` is Clone via Arc-wrapped dependencies
- Agent lock (`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock` (429 if busy)
- Heartbeat pre-filter: active hours (8-21 local), rate limits (1/hour, 3/day), skip if user messaged within 2h
- Failed sends flush: before each message processing, flushes up to 5 pending failed outbound sends

## Gateway (mika-gateway)

Stateless Telegram webhook router with Postgres customer registry:

- `POST /webhook/telegram` — Receives Telegram updates, validates secret token, routes to customer containers
- `POST /send` — Agent containers POST outbound messages for Telegram delivery (Bearer auth)
- `GET /health`, `/readyz` — K8s readiness probe (checks Postgres connectivity)
- `GET /livez` — K8s liveness probe (always 200)

Features:
- Constant-time secret validation (subtle crate)
- Atomic dedup via `last_update_id` column (prevents duplicate message processing)
- Concurrency control via semaphore (30 concurrent webhook handlers)
- Customer pairing via `/start <pairing_token>` deep links

## CLI (mika-cli)

Terminal UI chat interface with clap subcommands:

- `mika` — Interactive chat (ratatui TUI with Shift+Enter for newlines, PageUp/PageDown scroll)
- `mika ask "message"` — Send a single message, print response (use `-` for stdin)
- `mika status` — Health info
- `mika memory search "query"` — Search stored facts
- `mika reminders` — List reminders
- `mika skills list` — List installed skills
- `mika config edit` — Edit identity config
- `mika agents list/switch/create/clone/delete` — Manage agents
- `mika teams list/run/create/status/log/delete` — Team workflows

## Multi-Agent Support

- Global home directory: `~/.mika/`
- Agent homes: `~/.mika/agents/{name}/` (each with data/, skills/, logs/)
- Active agent tracked in `~/.mika/active_agent`
- CLI `--agent` flag overrides active agent
- Server discovers all agents on startup

## Silent Mode

Background tasks (heartbeat, reminders) where text output is NOT delivered to the user:
- Agent must use `send_message` tool explicitly to contact the user
- Separate `run_silent_agent` function with `SilentPromptContext`
- Reminder scheduler with recovery on startup (fires past-due reminders)

## Deployment

- Docker multi-stage builds with dependency layer caching
- `Dockerfile.agent` (~95MB) for per-customer containers
- `Dockerfile.gateway` (~90MB) for shared router
- Both run as non-root user `mika` (UID 1000)
- Release profile: LTO + strip for minimal binary size

## Configuration

- `config-rs` with `MIKA_` env prefix
- Required: `MIKA_ANTHROPIC_API_KEY`
- Optional: `MIKA_OPENAI_API_KEY` (Layer 3 vector search)
- Server mode: `MIKA_ROUTING_URL`, `MIKA_INTERNAL_TOKEN`
- `Settings` has manual `Debug` impl that redacts API keys
