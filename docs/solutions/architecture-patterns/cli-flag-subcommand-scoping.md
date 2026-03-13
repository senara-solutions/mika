---
title: Scope CLI flags to relevant subcommands via shared Args struct
module: mika-cli (cli.rs, main.rs)
severity: medium
tags:
  - cli
  - clap
  - ux
  - flag-scoping
  - compile-time-safety
date: 2026-03-13
related_issues:
  - "#102"
---

# Scope CLI flags to relevant subcommands via shared Args struct

## Problem

The `--agent` and `--team` flags were declared with `global = true` on the top-level `Cli` struct. Clap propagated them to every subcommand's `--help` output, including `setup`, `doctor`, and `teams` where they had no effect. Users were confused into thinking the flags did something on those subcommands.

## Root Cause

Clap's `global = true` makes a flag available at every position in the command hierarchy. There is no built-in mechanism to selectively show a flag on some subcommands but not others — it's all or nothing with `global`.

## Solution

### 1. Shared `AgentFlag` args struct

Created a reusable `#[derive(clap::Args)]` struct containing the `--agent` field:

```rust
#[derive(clap::Args, Clone, Debug)]
pub struct AgentFlag {
    /// Agent to use (overrides active agent)
    #[arg(long)]
    pub agent: Option<String>,
}
```

### 2. Selective flattening into subcommands

Each relevant subcommand's args struct includes `#[command(flatten)] pub agent_flag: AgentFlag`. Subcommands that should not accept `--agent` simply don't flatten it.

| Flag | Subcommands |
|------|-------------|
| `--agent` | chat, ask, memory, reminders, status, config, skills, mcp, tasks, agents |
| `--team` | chat only |
| Neither | setup, doctor, teams |

### 3. Dual-level flags for bare invocation

The `Cli` struct retains `--agent` and `--team` as non-global fields for the bare `mika --agent work` case (no subcommand). Resolution in `main.rs` merges both levels:

```rust
let agent_override = cli.command.as_ref()
    .and_then(|c| c.agent_override())
    .or(cli.agent.as_deref());
```

Subcommand-level takes priority over top-level.

### 4. Exhaustive match for compile-time safety

The `agent_override()` and `team_override()` methods on `Commands` explicitly list all variants instead of using a wildcard `_ => None`. This ensures adding a new `Commands` variant produces a compile error, forcing the developer to make a conscious scoping decision:

```rust
// No agent override — listed explicitly so adding a new Commands variant
// produces a compile error, forcing a conscious scoping decision.
Commands::Setup { .. } | Commands::Doctor(_) | Commands::Teams(_) => None,
```

## Key Decisions

- **Kept flags on `Cli` struct (non-global)** for backward compatibility with `mika --agent work` syntax. The alternative was removing them entirely, but that would break existing scripts.
- **`--session` left as `global = true`** — same pattern applies but was out of scope for issue #102.
- **Cross-level conflict not enforced** — `mika --agent work chat --agent research` silently uses the subcommand value. Clap cannot detect conflicts across argument scopes. The merge logic handles this correctly (subcommand wins).

## Prevention

When adding new subcommands to the `Commands` enum:

1. The exhaustive match arms in `agent_override()` and `team_override()` will produce a compile error
2. Decide whether the new subcommand needs `--agent` — if yes, flatten `AgentFlag` into its args struct and add a match arm
3. If not, add it to the `=> None` arm explicitly

## Related

- [Team TUI mode CLI integration](../integration-issues/team-tui-mode-cli-integration.md) — original `--team` flag implementation
- [Config key registry CLI management](config-key-registry-cli-management.md) — another CLI args pattern
