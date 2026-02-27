# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation on Kubernetes. Each customer gets their own agent container with plaintext SQLite storage on K8s encrypted volumes. A shared routing layer (Phase 2) will handle Telegram/WhatsApp and forward messages to the correct container.

**Current phase:** Phase 4 — Deployment infrastructure (Dockerfiles done, Helm charts + provisioning next).

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → Claude API → match stop_reason → execute tools or respond
- **LLM:** Claude (Sonnet 4.6 default) via direct reqwest calls to Messages API
- **Database:** SQLite via rusqlite (per-customer, plaintext on K8s encrypted volumes)
- **HTTP server:** Axum 0.8 (mika-server binary) with tower-http middleware
- **HTTP client:** reqwest 0.12 (Claude API client with typed errors and retry)
- **Async runtime:** tokio
- **Config:** config-rs with `MIKA_` env prefix
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev)

## Directory Structure

- `crates/mika-common/` — Shared library: config, Claude API client, logging, home directory
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, HTTP server binary
- `crates/mika-cli/` — TUI CLI binary (`mika`): ratatui chat interface, clap subcommands (status, memory, reminders, config, setup)
- `crates/mika-gateway/` — Gateway: Telegram webhook router, customer pairing, outbound relay (Postgres-backed)
- `config/` — Configuration files (default.toml; local.toml is gitignored)
- `docs/brainstorms/` — Decision brainstorm documents
- `docs/plans/` — Implementation plans
- `todos/` — Code review findings (tracked as markdown files)
- `.claude/commands/` — Claude Code slash commands (`/mika` — full dev workflow with doc audit)

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Data at rest:** Plaintext SQLite on K8s encrypted volumes. Per-customer container isolation. Case-insensitive COLLATE NOCASE on unique text columns.
- **Secrets:** `Settings` has manual `Debug` impl that redacts API key. API key errors are opaque.
- **Tools:** Each tool validates inputs (empty check + 10,000 char max). `ToolContext` contains `{ db, session_id, home_dir, core_memory_edit_count, is_onboarding, message_sender, embedding_client }`. Tool trait uses `#[async_trait]` (Send futures, required for `tokio::spawn` in server handlers).
- **Async DB:** `AsyncDatabase` wraps sync `Database` with dedicated OS thread + `mpsc` channel (closure-based dispatch). Clone-able, Send+Sync. Integrated into agent loop, tools, and scheduler.

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (~475 tests)
- `cargo run --bin mika` — Run TUI CLI (default: chat, or `mika status`, `mika memory`, etc.)
- `cargo run --bin mika-server` — Run HTTP server (requires `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN`)
- `cargo clippy` — Lint
- `cargo fmt` — Format
- `docker build -f Dockerfile.agent -t mika-agent:dev .` — Build agent container image
- `docker build -f Dockerfile.gateway -t mika-gateway:dev .` — Build gateway container image

## Architecture

- **One container per customer** on Kubernetes
- **Three-layer memory model:**
  - Layer 1: Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2000 token limit)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events — plaintext). Managed via `store_fact`, `update_fact`, `search_memory` tools.
  - Layer 3: Hybrid search (FTS5 full-text + sqlite-vec cosine similarity via Reciprocal Rank Fusion). Optional OpenAI embeddings (text-embedding-3-small, 512 dims). Graceful degradation: hybrid → FTS5-only → LIKE fallback. Indexed on store_fact/update_fact, backfilled on startup.
- **Agent loop:** Max 10 tool steps, 5-minute total timeout, 30s per-tool timeout
- **Typed Claude API errors:** `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- **Audit log:** `memory_events` table tracks all memory mutations per session
- **Conversation compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt (not message history). Runs inline post-turn in CLI.
- **Silent mode agent loop:** Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`.
- **MessageSender trait:** `#[async_trait]` with `Send + Sync` bounds for `Arc<dyn MessageSender>`. CLI prints to stdout. Server uses `GatewayMessageSender` (POST to gateway `/send` with retry + failed_sends fallback).
- **Reminder scheduler:** `ReminderScheduler` uses owned types (no lifetime params). `recover()` fires past-due reminders on startup via silent agent.
- **HTTP server (mika-server):** Axum-based with 3 endpoints: `/health` (no auth, K8s probes), `/message` (Bearer auth, 202 async), `/heartbeat` (Bearer auth, CronJob trigger). `AppState` is Clone via Arc-wrapped deps. Agent lock (`tokio::sync::Mutex<()>`) serializes agent loops with non-blocking `try_lock` (429 if busy).
- **Heartbeat pre-filter:** Active hours (8-21 local via chrono-tz), rate limits (1/hour, 3/day), skip if user messaged within 2h. All checks before acquiring Mutex.
- **Failed sends flush:** Before each message processing, flushes up to 5 pending failed outbound sends from DB.
- **Schema version:** 8 (v7 adds: skills system tables; v8 adds: search_content, fts_search FTS5, vec_search vec0 for Layer 3)
- **mika-gateway:** Telegram webhook router with Postgres customer registry. Endpoints: `/webhook/telegram` (inbound), `/send` (outbound relay), `/health` + `/readyz` + `/livez` (K8s probes). Stateless, env-var-only config.
- **Docker images:** Multi-stage builds with dependency layer caching. `Dockerfile.agent` (95MB) for per-customer containers. `Dockerfile.gateway` (90MB) for shared router. Both run as non-root user `mika` (UID 1000). Release profile: LTO + strip.

## Environment Variables

See `.env.example` for the full list. Required:
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key or OAuth subscription token. Auto-detected from prefix: `sk-ant-oat*` → OAuth bearer auth, otherwise → standard API key auth.

Optional (Layer 3 vector search):
- `MIKA_OPENAI_API_KEY` — OpenAI API key for embedding generation (enables vector similarity in hybrid search)

Server mode additionally requires:
- `MIKA_ROUTING_URL` — Gateway URL for outbound message delivery
- `MIKA_INTERNAL_TOKEN` — Shared secret for Bearer auth between gateway and agent

## Pending Work

- **Deployment:** Helm charts (mika-customer + mika-gateway), provision.sh + deprovision.sh, AWS Secrets Manager integration, K8s manifests
- **Future features:** WhatsApp channel adapter, morning briefings, admin API

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `/home/samidarko/workspace/senara-solutions/openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `/home/samidarko/workspace/senara-solutions/lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
