---
title: Scope --agent and --team CLI flags to relevant subcommands only
type: refactor
status: completed
date: 2026-03-13
---

# Scope --agent and --team CLI flags to relevant subcommands only

## Overview

Remove `global = true` from `--agent` and `--team` flags on the top-level `Cli` struct in `crates/mika-cli/src/cli.rs`. Add them explicitly to only the subcommands that use them via a shared `Args` struct pattern.

## Problem Statement

`--agent` and `--team` are declared with `global = true`, causing clap to display them in `--help` for every subcommand — including `setup`, `doctor`, and `teams` where they have no effect. This confuses users into thinking the flags do something on those subcommands.

## Proposed Solution

1. **Create a shared `AgentFlag` args struct** with the `--agent` field
2. **Flatten `AgentFlag`** into each relevant subcommand's fields/args struct
3. **Add `--team` directly to `Chat`** (only subcommand that uses it)
4. **Update `main.rs`** to extract agent/team from each command variant instead of from `Cli`
5. **Keep `--session` as `global = true`** (out of scope per issue #102, but note it has the same pattern)

### Flag scoping

| Flag | Subcommands |
|------|-------------|
| `--agent` | `chat`, `ask`, `memory`, `reminders`, `status`, `config`, `skills`, `mcp`, `tasks`, `agents` |
| `--team` | `chat` only |
| `--agent` NOT on | `setup`, `doctor`, `teams` |

### Implementation details

**`crates/mika-cli/src/cli.rs`:**

- Remove `agent` and `team` fields from `Cli` struct (keep `session` with `global = true`)
- Create shared args struct:

```rust
#[derive(Args, Clone, Debug)]
pub struct AgentFlag {
    /// Override the active agent
    #[arg(long)]
    pub agent: Option<String>,
}
```

- For commands with existing Args structs (`MemoryArgs`, `ReminderArgs`, `ConfigArgs`, `SkillsArgs`, `McpArgs`, `TaskArgs`, `AgentsArgs`): add `#[command(flatten)] pub agent_flag: AgentFlag`
- For `Chat`: restructure to include both `AgentFlag` (flattened) and `--team` field with `conflicts_with = "agent"`
- For `Ask`: add `AgentFlag` (flattened) alongside existing inline fields
- For `Status`: wrap in a `StatusArgs` struct or add inline fields

- Add helper method on `Commands`:

```rust
impl Commands {
    pub fn agent_override(&self) -> Option<&str> {
        match self {
            Commands::Chat(args) => args.agent_flag.agent.as_deref(),
            Commands::Ask(args) => args.agent_flag.agent.as_deref(),
            Commands::Memory(args) => args.agent_flag.agent.as_deref(),
            Commands::Reminders(args) => args.agent_flag.agent.as_deref(),
            Commands::Status(args) => args.agent_flag.agent.as_deref(),
            Commands::Config(args) => args.agent_flag.agent.as_deref(),
            Commands::Skills(args) => args.agent_flag.agent.as_deref(),
            Commands::Mcp(args) => args.agent_flag.agent.as_deref(),
            Commands::Tasks(args) => args.agent_flag.agent.as_deref(),
            Commands::Agents(args) => args.agent_flag.agent.as_deref(),
            _ => None, // Setup, Doctor, Teams — no agent override
        }
    }

    pub fn team_override(&self) -> Option<&str> {
        match self {
            Commands::Chat(args) => args.team.as_deref(),
            _ => None,
        }
    }
}
```

**`crates/mika-cli/src/main.rs`:**

- Replace `cli.agent` with `cli.command.as_ref().and_then(|c| c.agent_override()).map(String::from)` (or similar)
- Replace `cli.team` with `cli.command.as_ref().and_then(|c| c.team_override()).map(String::from)`
- For `setup` and `doctor`: use `init::resolve_active_agent()` directly (no `--agent` override)
- Restructure the match arms to extract fields from the new arg structs

### Commands needing restructure

| Command | Current structure | Change needed |
|---------|-------------------|---------------|
| `Chat` | No args struct (bare variant) | Create `ChatArgs` with `AgentFlag` + `team` |
| `Ask` | Inline fields | Create `AskArgs` with `AgentFlag` + existing fields |
| `Status` | No args struct (bare variant) | Create `StatusArgs` with `AgentFlag` |
| `Setup` | Inline fields | No change (no flag) |
| `Doctor` | `DoctorArgs` | No change (no flag) |
| `Agents` | `AgentsArgs` | Add `AgentFlag` flatten |
| `Teams` | `TeamsArgs` | No change (no flag) |
| `Memory` | `MemoryArgs` | Add `AgentFlag` flatten |
| `Reminders` | `ReminderArgs` | Add `AgentFlag` flatten |
| `Config` | `ConfigArgs` | Add `AgentFlag` flatten |
| `Skills` | `SkillsArgs` | Add `AgentFlag` flatten |
| `Mcp` | `McpArgs` | Add `AgentFlag` flatten |
| `Tasks` | `TaskArgs` | Add `AgentFlag` flatten |

## Acceptance Criteria

- [x] `mika setup --help` does NOT show `--agent` or `--team`
- [x] `mika doctor --help` does NOT show `--agent` or `--team`
- [x] `mika teams --help` does NOT show `--agent` or `--team`
- [x] `mika chat --help` shows both `--agent` and `--team`
- [x] `mika ask --help` shows `--agent` but NOT `--team`
- [x] `mika memory --help` shows `--agent` but NOT `--team`
- [x] `--agent` and `--team` remain mutually exclusive on `chat`
- [x] All existing tests pass (`cargo test`)
- [x] `cargo clippy` passes
- [x] Bare `mika` (no subcommand, defaults to chat) still accepts `--agent` and `--team`

## Context

- Related issue: #102
- Key file: `crates/mika-cli/src/cli.rs` (Cli struct, Commands enum)
- Key file: `crates/mika-cli/src/main.rs` (flag consumption, dispatch)
- Learning: Team TUI mode uses early branching in main.rs — `--team` check happens before agent resolution
- `--session` also has `global = true` but is out of scope for this issue
