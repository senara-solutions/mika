# Contributing to Mika

Mika is developed using [Claude Code](https://docs.anthropic.com/en/docs/claude-code) with the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin). We strongly recommend this workflow, but manual contributions are welcome too.

## Prerequisites

- **Rust** >= 1.91 (see `rust-version` in `Cargo.toml`)
- **jq** -- required by skill handler scripts
- **Claude Code** + **compound engineering plugin** (recommended, not required)

Build with `cargo build` and run tests with `cargo test`. Tests are fully mocked and do not require a `MIKA_ANTHROPIC_API_KEY`.

## Development Workflow with Claude Code

The recommended workflow uses the `/mika` slash command, which chains every step from planning through documentation:

```
/mika <description of what you want to build or fix>
```

This runs the following steps in order:

1. **Plan** (`/workflows:plan`) -- Research the codebase, design the approach, write a plan file to `docs/plans/`
2. **Work** (`/workflows:work`) -- Implement the plan with incremental commits and continuous testing
3. **Review** (`/workflows:review`) -- Multi-agent code review for quality and correctness
4. **Resolve TODOs** (`/compound-engineering:resolve_todo_parallel`) -- Address review findings tracked in `todos/`
5. **Doc Audit** (`/mika-doc-audit`) -- Update documentation based on code changes
6. **Compound** (`/workflows:compound`) -- Document the solution in `docs/solutions/` for institutional knowledge

The command uses a `/ralph-loop` wrapper internally to ensure all steps run to completion without stopping between them.

For documentation-only changes, you can run `/mika-doc-audit` directly instead of the full `/mika` workflow.

### Setup

1. Install [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
2. Install the [compound engineering plugin](https://github.com/EveryInc/compound-engineering-plugin)
3. Run `/mika` from the project root

## Manual Workflow

If you prefer not to use Claude Code:

```bash
# 1. Create a feature branch (type/description-kebab-case)
git checkout -b feat/my-feature

# 2. Make changes and run quality gates (these match CI exactly)
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test

# 3. Commit with conventional format (see below)
git commit -m "feat(agent): add new capability"

# 4. Push and open a PR
git push -u origin feat/my-feature
```

Branch types: `feat/`, `fix/`, `refactor/`, `docs/`, `chore/`.

## Commit Conventions

We use [Conventional Commits](https://www.conventionalcommits.org/) for automated changelog generation via [release-plz](https://release-plz.ieni.dev/). Use these prefixes:

**Appears in changelog:**

| Prefix | Changelog Group | Example |
|--------|----------------|---------|
| `feat` | Added | `feat(tui): add model switching` |
| `fix` | Fixed | `fix(agent): handle empty response` |
| `refactor` | Changed | `refactor: extract config module` |
| `perf` | Performance | `perf(search): optimize FTS5 queries` |
| `doc`/`docs` | Documentation | `doc: update architecture guide` |

**Skipped in changelog** (still valid):

| Prefix | Use for |
|--------|---------|
| `test` | Test additions or fixes |
| `ci` | CI/CD pipeline changes |
| `chore` | Dependency updates, tooling |
| `style` | Formatting (cargo fmt) |

Scopes are optional. Common scopes: `agent`, `tui`, `gateway`, `common`, `cli`.

## Testing

- Tests live inline in each module: `#[cfg(test)] mod tests`
- No API key is required -- tests are fully mocked
- Some tests use `serial_test` for isolation; respect `#[serial]` annotations
- Add tests for new functionality, covering validation, success paths, and edge cases

## Documentation

When your changes affect behavior, update the relevant docs. The `/mika-doc-audit` step handles this automatically when using the Claude Code workflow.

| What Changed | Update |
|-------------|--------|
| Significant feature or behavior changes | `CLAUDE.md` (project instructions) |
| Environment variables | `.env.example` |
| Architecture or agent loop | `docs/architecture.md` |
| Configuration or settings | `docs/configuration.md` |
| Skills system | `docs/skills.md` |
| TUI slash commands | `docs/slash-commands.md` |
| Deployment or Docker | `docs/deployment.md` |
| New user-facing features | `docs/getting-started.md` |
| Public API changes | `README.md` |

For significant architectural changes, add an ADR to `docs/adr/` following the existing sequential numbering and Context/Decision/Consequences format.

## Security Guidelines

- Never log API keys or secrets
- `Settings` has a manual `Debug` impl that redacts sensitive fields -- follow this pattern
- Child processes use `env_clear()` + allowlist to prevent `MIKA_*` env var leakage
- Handler scripts `unset` MIKA env vars before executing commands
- When adding tools that spawn processes, use the existing `env_clear()` pattern in `McpManager`

## License

By contributing to Mika, you agree that your contributions will be licensed under the [MIT License](LICENSE).
