# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation on Kubernetes. Each customer gets their own agent container with encrypted SQLite storage. A shared routing layer handles Telegram/WhatsApp and forwards messages to the correct container.

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → Claude API → match stop_reason → execute tools or respond
- **LLM:** Claude (Sonnet 4.5 default, Opus 4.6 for complex tasks) via direct reqwest calls
- **Per-customer DB:** SQLite + sqlite-vec (conversations, memory, embeddings)
- **Shared DB:** PostgreSQL 16 via sqlx (customers, channel mappings, invitations)
- **Encryption:** AES-256-GCM via `ring` for sensitive SQLite columns
- **HTTP:** axum 0.8 (routing layer), reqwest 0.12 (Claude API)
- **Async runtime:** tokio
- **Config:** config-rs with MIKA_ env prefix
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev)

## Directory Structure

- `crates/mika-common/` - Shared library: config, encryption, Claude client, logging, types
- `crates/mika-agent/` - Agent container: SQLite DB, agent loop, tools, prompt assembly, CLI binary
- `crates/mika-routing/` - Routing layer: axum HTTP server, PostgreSQL, channel adapters
- `docs/brainstorms/` - Decision brainstorm documents
- `docs/plans/` - Implementation plans
- `config/` - Configuration files (default.toml, local.toml)

## Conventions

- **Async by default:** All I/O operations use async/await with tokio
- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework

## Commands

- `cargo build` - Build all crates
- `cargo test` - Run all tests
- `cargo run --bin mika-cli` - Run CLI test harness
- `cargo clippy` - Lint
- `cargo fmt` - Format

## Architecture

- **One container per customer** on Kubernetes
- **Shared Telegram/WhatsApp bot** with stateless routing (no customer data in router)
- **Three-layer memory model:**
  - Layer 1: Core memory (always in context, agent-editable via tools)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events)
  - Layer 3: Vector search (sqlite-vec + FTS5 hybrid)
- **Proactive behavior:** tokio-cron-scheduler + heartbeat for follow-ups
- **Google Calendar:** Python sidecar service (kept separate)

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `/home/samidarko/workspace/senara-solutions/openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `/home/samidarko/workspace/senara-solutions/lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
