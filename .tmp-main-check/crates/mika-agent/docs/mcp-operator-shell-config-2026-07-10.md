# MCP config boundary — operator-shell scoped (mika#1737 sub-G)

**Sub-ticket G** of mika#1727. Ratified by mika-arch (session `0e01b314-4085-439e-95a4-ecc9cc103d6d`) as Option (a): MCP config is CLI/operator-side, edit-time, operator-shell scoped. This doc records the shipped surface + migration semantics.

## Ratified disposition

Verbatim from mika-arch's ratification (mika#1737 issue body):

> **C1 — Wrapper-doctrine precedent.** MCP server definitions are operator configuration, not runtime state. Same shape as `mika config set`.
>
> **C2 — YAGNI.** Absent a concrete load-bearing scenario for mid-session MCP mutation, Option (a) is the minimal viable boundary.
>
> **C3 — Session lifecycle alignment.** Spirit initializes MCP sessions at mika-session start based on the static config the CLI edited.
>
> **C4 — Structural simplicity.** Reuses existing config-file patterns; no new HTTP endpoints, no persistence in spirit.

## AC delta shipped

| AC | Status | Location |
|---|---|---|
| AC1 | ✅ Landed | `commands/mcp.rs` writes to operator-shell path via `McpConfig::save_operator_shell()` — same pattern as `mika config set` |
| AC2 | 🟡 Partial | `commands/mcp.rs` stops importing per-agent `mcp.json`. `chat.rs` and `ask.rs` still call `init::connect_mcp` for CLI-mode agent loops; full removal of `McpManager` from those sites is folded into mika#1727 when the CLI becomes a thin client of spirit |
| AC3 | ✅ Landed | Spirit's `server/mod.rs` reads from the same operator-shell path at `init_agent` time via `McpConfig::load_operator_shell()` |
| AC4 | ✅ Landed | Path detectable without CLI coordination: `mika_common::mcp_config_path::resolve_operator_mcp_config_path()` is the single source of truth. Resolution chain covers XDG-compliant + POSIX + env-override cases. Integration-tested at `crates/mika-agent/tests/mcp_operator_shell_config.rs` |
| AC5 | ✅ Landed | `McpConfig::migrate_from_agent_home_if_needed()` — one-shot copy from `{agent_home}/mcp.json`. Idempotent. Called from `commands/mcp.rs`, `init::connect_mcp`, `server::init_agent`, and `agents validate` |

## Resolution chain

The operator-shell path is resolved via `mika_common::mcp_config_path::resolve_operator_mcp_config_path()` with four tiers (first hit wins):

| Priority | Source | Path |
|---|---|---|
| 1 | Env var | `$MIKA_MCP_CONFIG` (absolute path override — load-bearing for tests + non-XDG hosts) |
| 2 | XDG env var | `$XDG_CONFIG_HOME/mika/mcp-servers.json` |
| 3 | POSIX HOME | `$HOME/.config/mika/mcp-servers.json` |
| 4 | CWD fallback | `./mcp-servers.json` (WARN-logged; callers should escalate) |

Format: JSON. Kept identical to the pre-existing per-agent `mcp.json` schema — the pre-migration config file can be hand-copied across the boundary without transformation.

## Migration semantics (AC5)

`McpConfig::migrate_from_agent_home_if_needed(agent_home: &Path) -> Result<bool>`:

- **When it fires**: operator-shell path does NOT exist AND `{agent_home}/mcp.json` DOES exist.
- **What it does**: copy contents to operator-shell path (create parents as needed; `0600` on Unix).
- **Idempotency**: once the operator-shell path exists, all subsequent invocations are no-ops (return `Ok(false)`) even if the legacy per-agent file still exists.
- **Multi-agent semantics**: the FIRST invocation across all agents wins. If several agents had distinct `mcp.json` files, later invocations do NOT merge — operators must hand-merge after the first migration. This is the deliberate operator-shell-scoped shape from AC3 (the pre-migration divergent-per-agent state was already an anti-pattern).
- **Failure mode**: IO errors are non-fatal for callers. Each call site logs-and-continues; the operator can hand-copy if needed.

## Call-site map

| Site | File | Behavior after this PR |
|---|---|---|
| CLI edit-time | `crates/mika-cli/src/commands/mcp.rs` | Runs migration, then `load_operator_shell` / `save_operator_shell` |
| CLI-mode agent loop | `crates/mika-cli/src/init.rs::connect_mcp` | Runs migration, then `load_operator_shell` (the `McpManager` import stays — fold-in with mika#1727) |
| Server-side session init | `crates/mika-agent/src/server/mod.rs` at `init_agent` | Runs migration, then `load_operator_shell` |
| Validation | `crates/mika-agent/src/validate.rs` | Runs migration, then `load_operator_shell` — surfaces the resolved path in the `[OK]` / `[FAIL]` diagnostic string |

## AC2 residual — CLI-side `McpManager` removal

`crates/mika-cli/src/commands/chat.rs` and `crates/mika-cli/src/commands/ask.rs` still import `mika_agent::mcp::McpManager` to thread into `AgentParams` for the CLI-mode agent loop. Per the audit doc §AC2 wrapper-doctrine, this is the surface that goes away when the CLI becomes a thin client of spirit (mika#1727). Removing the `McpManager` import in this PR would functionally regress CLI-mode MCP support before the thin-client refactor lands.

**Decision**: keep `init::connect_mcp` as an opaque helper for CLI-mode; it now reads from the operator-shell path via the same loader spirit uses. When mika#1727 removes the CLI-side agent loop, `connect_mcp` and its callers in `chat.rs`/`ask.rs` disappear together. This is a scope-preserving choice, not a doctrine dilution.

## Not in scope (explicit)

Per the ratified disposition:

- Dynamic add/remove during a session — C2 YAGNI, deferred until a concrete use case emerges. Can add without breaking changes later (env var + server-side endpoint could be added on top of the operator-shell shape).
- Any HTTP endpoint on spirit for MCP CRUD — Option (b) rejected.
- Runtime-state query for MCP servers (connected/errored/tool-loaded state). File a separate follow-up read-only endpoint ticket if TUI later wants this.
- Cross-tenant / per-agent MCP scoping — single-user shell shape in Phase 1. Multi-agent operators with divergent MCP needs are asked to hand-merge post-migration.

## Cross-links

- **Parent ticket**: `senara-solutions/mika#1727`.
- **Ratifying arch session**: `0e01b314-4085-439e-95a4-ecc9cc103d6d` (mika-arch, 2026-07-06).
- **Doctrinal precedent**: `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md` §AC3 (wrapper-doctrine subsection).
- **Sibling protocol docs**:
  - `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` (sub-C, mika#1733).
  - `crates/mika-agent/docs/ask-user-question-bridge-2026-07-10.md` (sub-D, mika#1734).
  - `crates/mika-agent/docs/session-messages-stream-verification-2026-07-10.md` (sub-F, mika#1736).
- **Fold-in ticket**: `senara-solutions/mika#1727` — closes AC2's residual `McpManager` import in `chat.rs`/`ask.rs` when the CLI becomes a thin client.
