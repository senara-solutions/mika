# Contributing to Mika

Mika is developed using [Claude Code](https://docs.anthropic.com/en/docs/claude-code) with the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin). We strongly recommend this workflow, but manual contributions are welcome too.

## Prerequisites

- **Rust** >= 1.91 (see `rust-version` in `Cargo.toml`)
- **jq** -- required by skill handler scripts
- **Claude Code** + **compound engineering plugin** (recommended, not required)

## Getting Started

```bash
# Clone the repository
git clone https://github.com/senara-solutions/mika.git
cd mika

# Build all crates
cargo build

# Run tests (no API key needed)
cargo test
```

Tests are fully mocked and do not require a `MIKA_ANTHROPIC_API_KEY`.

## Development Workflow with Claude Code

The recommended workflow uses the `/mika` slash command, which chains every step from planning through documentation:

```
/mika <description of what you want to build or fix>
```

This runs the following steps in order:

1. **Plan** (`/workflows:plan`) -- Research the codebase, design the approach, write a plan file
2. **Work** (`/workflows:work`) -- Implement the plan with incremental commits and continuous testing
3. **Review** (`/workflows:review`) -- Multi-agent code review for quality and correctness
4. **Resolve TODOs** (`/compound-engineering:resolve_todo_parallel`) -- Address code review findings in parallel
5. **Doc Audit** (`/mika-doc-audit`) -- Update documentation based on code changes
6. **Compound** (`/workflows:compound`) -- Document the solution for institutional knowledge

### Setup

1. Install [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
2. Install the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin)
3. Run `/mika` from the project root

## Manual Workflow

If you prefer not to use Claude Code:

```bash
# 1. Create a feature branch
git checkout -b feat/my-feature

# 2. Make changes and run quality gates
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

# 3. Commit with conventional format (see below)
git commit -m "feat(agent): add new capability"

# 4. Push and open a PR
git push -u origin feat/my-feature
```

## Branch Naming

Use `type/description-kebab-case`:

- `feat/add-web-search` -- new feature
- `fix/reminder-timezone` -- bug fix
- `refactor/extract-api-client` -- refactoring
- `docs/update-architecture` -- documentation

## Commit Conventions

We use [Conventional Commits](https://www.conventionalcommits.org/) for automated changelog generation via [release-plz](https://release-plz.ieni.dev/). Use these prefixes:

**Appears in changelog:**

| Prefix | Changelog Group | Example |
|--------|----------------|---------|
| `feat` | Added | `feat(tui): add model switching` |
| `fix` | Fixed | `fix(agent): handle empty response` |
| `refactor` | Changed | `refactor: extract config module` |
| `perf` | Performance | `perf(search): optimize FTS5 queries` |
| `doc` | Documentation | `doc: update architecture guide` |

**Skipped in changelog** (still valid):

| Prefix | Use for |
|--------|---------|
| `test` | Test additions or fixes |
| `ci` | CI/CD pipeline changes |
| `chore` | Dependency updates, tooling |
| `style` | Formatting (cargo fmt) |

Scopes are optional. Common scopes: `agent`, `tui`, `gateway`, `common`, `cli`.

## Quality Gates

Every PR must pass these checks (matching CI exactly):

```bash
cargo fmt --all -- --check       # Formatting
cargo clippy --all-targets --all-features -- -D warnings  # Linting (warnings are errors)
cargo test                       # All tests pass
```

Run these locally before pushing.

## Testing

- Tests live inline in each module: `#[cfg(test)] mod tests`
- No API key is required -- tests are fully mocked
- Some tests use `serial_test` for isolation; respect `#[serial]` annotations
- Add tests for new functionality, covering validation, success paths, and edge cases

## Documentation

When your changes affect behavior, update the relevant docs:

| What Changed | Update |
|-------------|--------|
| Any code change | `CLAUDE.md` (project instructions) |
| Environment variables | `.env.example` |
| Architecture or agent loop | `docs/architecture.md` |
| Configuration or settings | `docs/configuration.md` |
| Skills system | `docs/skills.md` |
| TUI slash commands | `docs/slash-commands.md` |
| Deployment or Docker | `docs/deployment.md` |
| New user-facing features | `docs/getting-started.md` |
| Public API changes | `README.md` |

The `/mika-doc-audit` step handles this automatically when using the Claude Code workflow.

## Architecture Decision Records

For significant architectural changes, add an ADR to `docs/adr/`:

- Follow the existing numbering: `006-your-decision.md`
- Use the format: Context, Decision, Consequences
- See existing ADRs for examples

## Security Guidelines

- Never log API keys or secrets
- `Settings` has a manual `Debug` impl that redacts sensitive fields -- follow this pattern
- Child processes use `env_clear()` + allowlist to prevent `MIKA_*` env var leakage
- Handler scripts `unset` MIKA env vars before executing commands
- When adding tools that spawn processes, use the existing `env_clear()` pattern in `McpManager`

## Project Documentation

| Document | Description |
|----------|-------------|
| [Architecture](docs/architecture.md) | System design, agent loop, memory model |
| [Getting Started](docs/getting-started.md) | Installation, first run, CLI commands |
| [Skills](docs/skills.md) | Creating and managing skills |
| [Configuration](docs/configuration.md) | Settings reference, directory layout |
| [Slash Commands](docs/slash-commands.md) | TUI command reference |
| [Deployment](docs/deployment.md) | Docker images, container deployment |

## License

By contributing to Mika, you agree that your contributions will be licensed under the [MIT License](LICENSE).
