# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation on Kubernetes. Each customer gets their own agent container with plaintext SQLite storage on K8s encrypted volumes. A shared routing layer (Phase 2) will handle Telegram/WhatsApp and forward messages to the correct container.

**Current phase:** Phase 1 — agent core with CLI test harness.

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → Claude API → match stop_reason → execute tools or respond
- **LLM:** Claude (Sonnet 4.6 default) via direct reqwest calls to Messages API
- **Database:** SQLite via rusqlite (per-customer, plaintext on K8s encrypted volumes)
- **HTTP:** reqwest 0.12 (Claude API client with typed errors and retry)
- **Async runtime:** tokio
- **Config:** config-rs with `MIKA_` env prefix
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev)

## Directory Structure

- `crates/mika-common/` — Shared library: config, Claude API client, logging, home directory
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, CLI binary
- `config/` — Configuration files (default.toml; local.toml is gitignored)
- `docs/brainstorms/` — Decision brainstorm documents
- `docs/plans/` — Implementation plans
- `todos/` — Code review findings (tracked as markdown files)

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Data at rest:** Plaintext SQLite on K8s encrypted volumes. Per-customer container isolation. Case-insensitive COLLATE NOCASE on unique text columns.
- **Secrets:** `Settings` has manual `Debug` impl that redacts API key. API key errors are opaque.
- **Tools:** Each tool validates inputs (empty check + 10,000 char max). `ToolContext` contains `{ db, session_id, home_dir, core_memory_edit_count, is_onboarding, message_sender }`. Tool trait uses `#[async_trait(?Send)]` (futures are NOT Send).
- **Async DB:** `AsyncDatabase` wraps sync `Database` with `Arc<Mutex<Database>>` + `tokio::task::spawn_blocking`. Clone-able, Send+Sync. Not yet integrated into agent loop (Phase 2).

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (127 tests)
- `cargo run --bin mika-cli` — Run CLI test harness
- `cargo clippy` — Lint
- `cargo fmt` — Format

## Architecture

- **One container per customer** on Kubernetes
- **Three-layer memory model:**
  - Layer 1: Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2000 token limit)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events — plaintext)
  - Layer 3: Vector search (sqlite-vec + FTS5 hybrid — not yet implemented)
- **Agent loop:** Max 10 tool steps, 5-minute total timeout, 30s per-tool timeout
- **Typed Claude API errors:** `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- **Audit log:** `memory_events` table tracks all memory mutations per session
- **Conversation compaction:** Threshold-based (50 messages). Keeps 20 most recent, summarizes older via Claude API. Summary injected into system prompt (not message history). Runs inline post-turn in CLI.
- **Silent mode agent loop:** Background tasks (heartbeat, reminders) where text output is NOT delivered. Agent must use `send_message` tool explicitly. Separate `run_silent_agent` function with `SilentPromptContext`.
- **MessageSender trait:** `#[async_trait(?Send)]` for outbound messaging. CLI prints to stdout. Phase 2 will add Telegram/WhatsApp senders.
- **Reminder scheduler:** `ReminderScheduler::recover()` fires past-due reminders on startup via silent agent. Future reminders logged but not timer-scheduled (Phase 2).
- **Schema version:** 5 (v5 adds: reminders, heartbeat_sends, customer_config, failed_sends tables; compacted_through_id column on conversations)

## Environment Variables

See `.env.example` for the full list. Required:
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key

## Pending Work

- **Phase 2 — HTTP server:** Integrate AsyncDatabase into agent loop, add Telegram/WhatsApp channel adapters, timer-based reminder scheduling, gateway routing
- `todos/027-ready-p1-sync-sqlite-blocking-tokio.md` — ~~Wrap sync SQLite in async~~ Done: `async_db.rs` created, integration deferred to Phase 2

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `/home/samidarko/workspace/senara-solutions/openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `/home/samidarko/workspace/senara-solutions/lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
