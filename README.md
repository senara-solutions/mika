# Mika

AI executive assistant with persistent memory and per-customer container isolation.

Mika is a conversation-first AI assistant that remembers what you tell it, tracks your commitments, learns your preferences, and keeps notes on the people in your life. It runs locally as a terminal UI or scales to multi-customer container deployments with Telegram integration.

## Architecture

```
CLI Mode (local)                    Hosted Mode (containers)

  +---------+                         +----------+
  |  mika   |                         | Telegram |
  |  (TUI)  |                         +----+-----+
  +----+----+                              |
       |                            +------v-------+
  +----v----+                       |   Gateway    |
  | SQLite  |                       |  (Postgres)  |
  +---------+                       +------+-------+
       |                                   |
  +----v--------+            +-------------+-------------+
  |  Claude API |            |             |             |
  +-------------+     +------v---+  +------v---+  +------v---+
                      | Agent A  |  | Agent B  |  | Agent C  |
                      | +SQLite  |  | +SQLite  |  | +SQLite  |
                      +----+-----+  +----+-----+  +----+-----+
                           |             |             |
                      +----v-------------v-------------v----+
                      |            Claude API               |
                      +-------------------------------------+
```

Each customer gets an isolated container with their own SQLite database. No customer data is shared.

## Features

- **Persistent memory** -- Core memory always in context, updated by the agent itself
- **Structured knowledge** -- Tracks people, commitments, preferences, and events
- **Skills system** -- Extensible filesystem-based tool registry (`~/.mika/skills/`)
- **Proactive heartbeat** -- Check-ins via silent agent loop with rate limiting
- **Reminders** -- Time-based reminders with recovery on restart
- **Conversation compaction** -- Automatic summarization of old messages
- **Slash commands** -- 13 client-side commands with tab-completion in the TUI
- **Multi-channel** -- CLI (local) and Telegram (hosted) with WhatsApp planned
- **Per-customer isolation** -- One container per customer

## Quick Start

```bash
# Clone and build
git clone https://github.com/senara-solutions/mika.git
cd mika
cargo build --release

# Set your Anthropic credential (API key or Claude subscription token)
export MIKA_ANTHROPIC_API_KEY=sk-ant-...

# Run (auto-setup on first launch)
cargo run --bin mika
```

On first run, Mika creates `~/.mika/` with default configuration, personality, and builtin skills, then opens an interactive chat.

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust (edition 2024) |
| LLM | Claude (Sonnet 4.6 default) via direct API |
| Database | SQLite via rusqlite (per-customer) |
| HTTP server | Axum 0.8 with tower-http |
| TUI | ratatui + tui-textarea + crossterm |
| Async runtime | tokio |
| Config | config-rs with `MIKA_` env prefix |

## Project Structure

```
crates/
  mika-common/     Shared: config, Claude API client, logging, home directory
  mika-agent/      Agent: SQLite DB, agent loop, tools, skills, HTTP server (mika-server)
  mika-gateway/    Telegram webhook router: Postgres customer registry, message routing
  mika-cli/        TUI CLI (mika): ratatui chat, clap subcommands, slash commands
```

## Tools

The agent has 8 tools organized into 3 skills:

| Skill | Tools | Description |
|-------|-------|-------------|
| Memory | `update_core_memory`, `store_fact`, `search_memory`, `update_fact` | Persistent memory management |
| Reminders | `create_reminder`, `list_reminders`, `cancel_reminder` | Time-based reminders |
| Messaging | `send_message` | Proactive outbound messages |

## Development

```bash
cargo build          # Build all crates
cargo test           # Run tests (226 tests)
cargo clippy         # Lint
cargo fmt            # Format
cargo run --bin mika # Run TUI CLI
```

## Documentation

| Document | Audience | Description |
|----------|----------|-------------|
| [Getting Started](docs/getting-started.md) | End users | Installation, first run, CLI commands |
| [Architecture](docs/architecture.md) | Developers | System design, agent loop, memory model |
| [Skills](docs/skills.md) | End users | Creating and managing skills |
| [Configuration](docs/configuration.md) | All | Settings reference, directory layout |
| [Slash Commands](docs/slash-commands.md) | End users | TUI command reference |
| [Deployment](docs/deployment.md) | Operators | Docker images, container deployment |

## Current Status

**Phase 4** -- Deployment infrastructure. Dockerfiles are complete. Gateway with Telegram integration is operational. CI/CD pipeline next.

## License

MIT
