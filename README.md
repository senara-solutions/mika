# Mika

AI executive assistant with persistent memory, per-customer container isolation, and encrypted local storage.

## What is Mika?

Mika is a conversation-first AI assistant designed for executives. It remembers what you tell it, tracks your commitments, learns your preferences, and keeps notes on the people in your life — all stored in an encrypted SQLite database that only you can access.

## Architecture

```
                  +-----------------+
                  | Telegram / WhatsApp |   (Phase 2)
                  +--------+--------+
                           |
                  +--------v--------+
                  |  Routing Layer  |   (Phase 2)
                  +--------+--------+
                           |
            +--------------+--------------+
            |              |              |
    +-------v------+ +----v-----+ +------v------+
    | Customer A   | | Customer B| | Customer C  |
    | mika-agent   | | mika-agent| | mika-agent  |
    | + SQLite     | | + SQLite  | | + SQLite    |
    +--------------+ +----------+ +-------------+
```

Each customer gets an isolated container with their own encrypted SQLite database. No customer data is shared.

## Features

- **Persistent memory** — Core memory always in context, updated by the agent itself
- **People tracking** — Remembers names, relationships, and notes about contacts
- **Commitment tracking** — Tracks tasks and deadlines with deduplication
- **Preference learning** — Stores how you like things done
- **Field-level encryption** — All PII encrypted with AES-256-GCM, HMAC lookups for queries
- **Secret protection** — Encryption keys zeroized on drop, secrets redacted from logs

## Quick Start

```bash
# Set required environment variables
export MIKA_ANTHROPIC_API_KEY=sk-ant-...
export MIKA_ENCRYPTION_KEY=$(openssl rand -hex 32)

# Run the CLI
cargo run --bin mika-cli
```

See `.env.example` for all configuration options.

## Project Structure

```
crates/
  mika-common/    Shared: config, encryption, Claude API client, logging
  mika-agent/     Agent: SQLite DB, agent loop, tools, prompt, CLI binary
config/
  default.toml    Default configuration values
```

## Development

```bash
cargo build       # Build
cargo test        # Run tests (32 tests)
cargo clippy      # Lint
cargo fmt         # Format
```

## Tools

The agent has four tools it can use during conversation:

| Tool | Purpose |
|------|---------|
| `update_core_memory` | Update persistent memory (always visible in system prompt) |
| `upsert_person` | Remember a person mentioned in conversation |
| `add_commitment` | Track a task or commitment |
| `set_preference` | Store a user preference |

## License

MIT
