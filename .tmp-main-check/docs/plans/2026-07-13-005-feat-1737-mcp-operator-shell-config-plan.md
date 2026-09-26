---
type: feat
issue: 1737
title: MCP operator-shell scoped config path + migration (mika#1737 sub-G)
status: draft
---

# Plan — mika#1737 sub-G MCP operator-shell config

## Ticket

mika#1737 — sub-G of mika#1727 (CLI thin-client refactor). Ratified by mika-arch
(session `0e01b314-4085-439e-95a4-ecc9cc103d6d`) as Option (a): MCP config is
CLI/operator-side, edit-time, operator-shell scoped. This ticket ships the
resolver, config load/save on the new path, one-shot per-agent migration, and
adjusts CLI + spirit + validation call-sites.

## Problem

Before this change, MCP server config was per-agent at `{agent_home}/mcp.json`.
That shape has three issues:

- **Divergent state across agents.** Multi-agent operators (e.g., mika-dev +
  mika-qa + mika-arch on the same host) had to hand-edit N files to add one
  MCP server.
- **No single source of truth.** CLI `mika mcp add`, spirit `init_agent`, and
  `agents validate` all read the same relative path but had no shared resolver,
  so future path changes would silently drift.
- **Coupled to the per-agent `home` layout** — a doctrine mismatch with the
  wrapper doctrine, which places CLI/operator config next to `mika config set`.

## Scope

Ship the operator-shell-scoped MCP config surface:

1. `mika_common::mcp_config_path` — sole-source resolver with a four-tier
   chain (env override, XDG, POSIX HOME, CWD fallback) and a `McpConfigPathSource`
   tag so callers can WARN on the fallback tier.
2. `McpConfig::load_operator_shell` / `save_operator_shell` — replace per-agent
   `load(agent_home)` / `save(agent_home)` at every write-site.
3. `McpConfig::migrate_from_agent_home_if_needed(agent_home)` — one-shot copy
   of legacy `{agent_home}/mcp.json` to the operator-shell path if the
   operator-shell path does not yet exist. Idempotent, non-fatal on IO error.
4. Call-site fanout: `crates/mika-cli/src/commands/mcp.rs`,
   `crates/mika-cli/src/init.rs::connect_mcp`,
   `crates/mika-agent/src/server/mod.rs::init_agent`,
   `crates/mika-agent/src/validate.rs::validate_agent`.
5. Integration tests at `crates/mika-agent/tests/mcp_operator_shell_config.rs`.
6. Doc: `crates/mika-agent/docs/mcp-operator-shell-config-2026-07-10.md`.
7. Repair `crates/mika-agent/src/validate.rs::tests::test_validate_agent_bad_mcp`
   to match the new operator-shell path and add env-isolation via
   `MIKA_MCP_CONFIG` + `#[serial]` (the original test wrote bad JSON to
   `{agent_home}/mcp.json` and asserted a substring `mcp.json failed to parse`
   which no longer describes the new diagnostic — and, worse, would migrate
   malformed JSON into the developer's real `~/.config/mika/mcp-servers.json`).

## Acceptance criteria

- [ ] `mika_common::mcp_config_path::resolve_operator_mcp_config_path()` returns
  `(PathBuf, McpConfigPathSource)` with correct precedence: `MIKA_MCP_CONFIG` >
  `$XDG_CONFIG_HOME/mika/mcp-servers.json` > `$HOME/.config/mika/mcp-servers.json`
  > `./mcp-servers.json`. Never fails. Unit-tested with each tier isolated via
  `#[serial]` env manipulation.
- [ ] `McpConfig::save_operator_shell()` writes to the resolved path, creates
  parent directories, sets `0600` on Unix. `load_operator_shell()` returns an
  empty config when the file is missing (fresh install boots clean) and returns
  `Err` only when the file exists but is malformed.
- [ ] `McpConfig::migrate_from_agent_home_if_needed(agent_home)` copies
  `{agent_home}/mcp.json` to the operator-shell path IFF the operator-shell
  path does not yet exist AND the per-agent path exists. Idempotent — later
  invocations across all agents are no-ops once the operator-shell path is
  populated. IO errors are non-fatal and returned as `Err`; callers
  log-and-continue.
- [ ] All four call-sites (CLI `commands/mcp.rs`, CLI `init.rs::connect_mcp`,
  spirit `server::init_agent`, `validate.rs::validate_agent`) call
  `migrate_from_agent_home_if_needed` before `load_operator_shell`; migration
  failures are logged at WARN and do NOT abort the caller.
- [ ] `validate_agent` diagnostic strings surface the resolved operator-shell
  path in both the `[OK]` and `[FAIL]` cases.
- [ ] `test_validate_agent_bad_mcp` passes with the new behavior: bad JSON at
  the operator-shell path (overridden via `MIKA_MCP_CONFIG` for hermetic
  isolation) produces a Fail diagnostic containing `mcp-servers.json` and
  `failed to parse`. Test is marked `#[serial_test::serial]` and cleans up
  its env-var override. No test writes into the developer's real
  `~/.config/mika/mcp-servers.json`.
- [ ] Integration tests in `crates/mika-agent/tests/mcp_operator_shell_config.rs`
  cover: (a) save/load roundtrip, (b) missing-file returns empty, (c)
  migration populates on first call, (d) migration is idempotent when target
  exists, (e) migration is a no-op on fresh install (no per-agent file), (f)
  headers with secret values roundtrip.

## Definition of Done

- All AC boxes checked.
- `cargo test -p mika-agent --lib validate::tests::test_validate_agent_bad_mcp`
  passes locally.
- `cargo test -p mika-agent --test mcp_operator_shell_config` passes locally.
- Class-A CI check green on PR #1764.
- Class-B "Pipeline Artifacts" gate green (this plan doc satisfies the
  picked-plan requirement).
- Doc `crates/mika-agent/docs/mcp-operator-shell-config-2026-07-10.md` is
  present on the branch and reflects the shipped AC delta table.

## References

- Parent ticket: mika#1727 (CLI thin-client refactor).
- Sub-ticket: mika#1737 sub-G.
- Ratifying arch session: `0e01b314-4085-439e-95a4-ecc9cc103d6d` (mika-arch,
  2026-07-06).
- Doctrinal precedent:
  `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md`
  § AC3 wrapper-doctrine subsection.
- Shipped doc: `crates/mika-agent/docs/mcp-operator-shell-config-2026-07-10.md`.
- Sibling sub-tickets: mika#1733 (sub-C permission decision), mika#1734
  (sub-D AskUserQuestion bridge), mika#1736 (sub-F session-messages stream).
