# Mika

AI executive assistant with persistent memory and per-customer container isolation.

## What is Mika?

Mika is a conversation-first AI assistant designed for executives. It remembers what you tell it, tracks your commitments, learns your preferences, and keeps notes on the people in your life — all stored in a per-customer SQLite database on Kubernetes encrypted volumes.

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

Each customer gets an isolated container with their own SQLite database on an encrypted volume. No customer data is shared.

## Features

- **Persistent memory** — Core memory always in context, updated by the agent itself
- **People tracking** — Remembers names, relationships, and notes about contacts
- **Commitment tracking** — Tracks tasks and deadlines with deduplication
- **Preference learning** — Stores how you like things done
- **Secret protection** — API keys redacted from logs

## Quick Start

```bash
# Set required environment variables
export MIKA_ANTHROPIC_API_KEY=sk-ant-...

# Run the CLI
cargo run --bin mika-cli
```

See `.env.example` for all configuration options.

## Project Structure

```
crates/
  mika-common/    Shared: config, Claude API client, logging, home directory
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
