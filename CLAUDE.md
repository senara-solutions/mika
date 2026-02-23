# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant with per-customer container isolation on Kubernetes. Each customer gets their own agent container with encrypted SQLite storage. A shared routing layer (Phase 2) will handle Telegram/WhatsApp and forward messages to the correct container.

**Current phase:** Phase 1 — agent core with CLI test harness.

## Stack

- **Language:** Rust (edition 2024)
- **Agent engine:** Explicit Rust loop (no framework) — retrieve context → build prompt → Claude API → match stop_reason → execute tools or respond
- **LLM:** Claude (Sonnet 4.6 default) via direct reqwest calls to Messages API
- **Database:** SQLite via rusqlite (per-customer, encrypted at field level)
- **Encryption:** AES-256-GCM via `ring` (cached LessSafeKey, zeroized on drop), HMAC-SHA256 for lookups
- **HTTP:** reqwest 0.12 (Claude API client with typed errors and retry)
- **Async runtime:** tokio
- **Config:** config-rs with `MIKA_` env prefix
- **Logging:** tracing + tracing-subscriber (JSON for prod, pretty for dev)

## Directory Structure

- `crates/mika-common/` — Shared library: config, encryption (AES + HMAC), Claude API client, logging
- `crates/mika-agent/` — Agent container: SQLite DB, agent loop, tools, prompt assembly, CLI binary
- `config/` — Configuration files (default.toml; local.toml is gitignored)
- `docs/brainstorms/` — Decision brainstorm documents
- `docs/plans/` — Implementation plans
- `todos/` — Code review findings (tracked as markdown files)

## Conventions

- **Error handling:** `anyhow::Result` for application code, `thiserror` for library errors (e.g., `CryptoError`, `ClaudeApiError`)
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Edition 2024:** `unsafe` blocks required for `std::env::set_var` etc.
- **Testing:** `#[cfg(test)] mod tests` inline in each module, `cargo test` to run
- **No framework:** The agent loop is a plain Rust async function, not a framework
- **Encryption:** All PII fields (names, relationships, categories, notes, messages) encrypted with AES-256-GCM. HMAC-SHA256 hashes for UNIQUE lookups on encrypted columns.
- **Secrets:** `EncryptionKey` uses `ZeroizeOnDrop`. `Settings` has manual `Debug` impl that redacts secrets. API key errors are opaque.
- **Tools:** Each tool validates inputs (empty check + 10,000 char max). `ToolContext` contains only `{ db }`.

## Commands

- `cargo build` — Build all crates
- `cargo test` — Run all tests (32 tests)
- `cargo run --bin mika-cli` — Run CLI test harness
- `cargo clippy` — Lint
- `cargo fmt` — Format

## Architecture

- **One container per customer** on Kubernetes
- **Three-layer memory model:**
  - Layer 1: Core memory (always in system prompt, agent-editable via `update_core_memory` tool, 2000 token limit)
  - Layer 2: Structured facts (People, Commitments, Preferences, Events — encrypted at rest)
  - Layer 3: Vector search (sqlite-vec + FTS5 hybrid — not yet implemented)
- **Agent loop:** Max 10 tool steps, 5-minute total timeout, 30s per-tool timeout
- **Typed Claude API errors:** `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- **Decryption resilience:** Failed decryptions logged at WARN level, startup key check

## Environment Variables

See `.env.example` for the full list. Required:
- `MIKA_ANTHROPIC_API_KEY` — Anthropic API key
- `MIKA_ENCRYPTION_KEY` — 64 hex chars (32 bytes) for AES-256-GCM

## Pending Work

- `todos/027-pending-p1-sync-sqlite-blocking-tokio.md` — Wrap sync SQLite in async (needed before Phase 2 HTTP server)

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `/home/samidarko/workspace/senara-solutions/openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `/home/samidarko/workspace/senara-solutions/lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
