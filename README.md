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
- **Skills system** -- Extensible filesystem-based tool registry with Git-based marketplace (`mika skills install`)
- **Proactive heartbeat** -- Check-ins via silent agent loop with rate limiting
- **Reminders** -- Time-based reminders with recovery on restart
- **Conversation compaction** -- Automatic summarization of old messages
- **Slash commands** -- 20 client-side commands with shell-like Tab completion and argument completers
- **Multi-channel** -- CLI (local) and Telegram (hosted) with WhatsApp planned
- **Per-customer isolation** -- One container per customer

## Quick Start

### Install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/senara-solutions/mika/main/install.sh | sh
```

Or install from [crates.io](https://crates.io/crates/mika-cli): `cargo install mika-cli`

Pre-built binaries are also available on the [Releases page](https://github.com/senara-solutions/mika/releases).

### Run

```bash
# Set your Anthropic API key (or see docs for other providers)
export MIKA_ANTHROPIC_API_KEY=sk-ant-...

# Run (auto-setup on first launch)
mika
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

The agent has 23 builtin tools and 6 conditional management tools:

| Category | Tools | Description |
|----------|-------|-------------|
| Memory | `update_core_memory`, `store_fact`, `search_memory`, `update_fact` | Persistent memory management |
| Reminders | `create_reminder`, `list_reminders`, `cancel_reminder` | Time-based reminders |
| Tasks | `create_task`, `cancel_task`, `complete_task`, `get_task`, `list_tasks` | Scheduled task management (callback, time, recurring triggers) |
| Files | `write_agent_file`, `read_agent_file`, `list_agent_files` | Read/write files in the agent's home directory |
| Skills | `create_skill`, `delete_skill`, `list_skills`, `toggle_skill`, `update_skill` | Skill registry management |
| Config | `get_config`, `set_config` | Customer configuration |
| Messaging | `send_message` | Proactive outbound messages |
| Management | `list_agents`, `delegate_task`, `list_teams`, `run_team`, `get_team_status`, `get_team_history` | Multi-agent delegation and team workflows (registered only when multiple agents or teams exist) |

## Development

```bash
cargo build          # Build all crates
cargo test           # Run tests (~1169 tests)
cargo clippy         # Lint
cargo fmt            # Format
cargo run --bin mika # Run TUI CLI
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development workflow, commit conventions, and quality gates.

## Documentation

| Document | Audience | Description |
|----------|----------|-------------|
| [Getting Started](docs/getting-started.md) | End users | Installation, first run, CLI commands |
| [Architecture](docs/architecture.md) | Developers | System design, agent loop, memory model |
| [Skills](docs/skills.md) | End users | Creating and managing skills |
| [Configuration](docs/configuration.md) | All | Settings reference, directory layout |
| [Slash Commands](docs/slash-commands.md) | End users | TUI command reference |
| [Deployment](docs/deployment.md) | Operators | Docker images, container deployment |
| [Contributing](CONTRIBUTING.md) | Contributors | Development workflow, conventions, quality gates |

## Versioning

Mika follows [Conventional Commits](https://www.conventionalcommits.org/) with automated releases via [release-plz](https://release-plz.ieni.dev/).

**Pre-1.0 breaking changes policy:** Until v1.0, breaking changes do not trigger a major version bump. They are shipped as minor or patch releases. Each PR that introduces a breaking change must document the required manual migration steps in its description so users can act on them during upgrade.

## Current Status

**Phase 4** -- Deployment infrastructure. Dockerfiles, CI/CD pipeline, and automated releases are complete. Gateway with Telegram integration is operational.

## License

MIT
