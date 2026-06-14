# Plan: Consistent `--format` flag across all list/query commands + Yaml variant

**Issue:** mika issue#371
**Type:** Enhancement
**Branch:** `feat/371/consistent-format-flag-across-all-list`

## Summary

Add a `Yaml` variant to the `OutputFormat` enum and add `--format` support to the few remaining list/query subcommands that lack it. The issue's audit table is stale — most commands already have `--format text|json`. The actual remaining work is smaller than described.

## Current State (Audit)

### Commands that ALREADY have `--format text|json`

- `mika ask` — `cli.rs:202`
- `mika status` — `cli.rs:251`
- `mika agents list` — `cli.rs:289`
- `mika agents validate` — `cli.rs:323`
- `mika skills list` — `cli.rs:417`
- `mika skills validate` — `cli.rs:494`
- `mika skills variants {list,status,diff,reflect,validate}` — `cli.rs:531-571`
- `mika teams list` — `cli.rs:354`
- `mika teams status` — `cli.rs:369`
- `mika teams log` — `cli.rs:377`
- `mika teams validate` — `cli.rs:396`
- `mika memory search` — `cli.rs:600`
- `mika reminders list` — `cli.rs:634`
- `mika reminders get` — `cli.rs:639`
- `mika tasks list` — `cli.rs:662`
- `mika tasks get` — `cli.rs:667`
- `mika config list` — `cli.rs:717`
- `mika provider` — `cli.rs:781`
- `mika model` — `cli.rs:819`
- `mika logs` — `cli.rs:832`
- `mika webhook list-dead` — `cli.rs:1024`
- `mika kg status` — `cli.rs:956`
- `mika kg list-agents` — `cli.rs:962`
- `mika kg purge` — `cli.rs:972`
- `mika kg validate` — `cli.rs:991`

### Commands that NEED `--format`

| Command | File | Notes |
|---------|------|-------|
| `mika memory` (bare) | `memory.rs:12-26` | Shows all core memory blocks |
| `mika memory people` | `memory.rs:116-133` | Lists tracked people |
| `mika memory commitments` | `memory.rs:135-150` | Lists commitments by status |
| `mika memory preferences` | `memory.rs:152-162` | Lists preferences |
| `mika memory events` | `memory.rs:164-179` | Lists events |
| `mika mcp list` | `mcp.rs:38-74` | Lists MCP servers |

### Commands correctly WITHOUT `--format`

These are action/mutation commands where structured output is not meaningful:

- `mika memory reset` — resets a memory block (confirmation message)
- `mika reminders cancel` — cancels a reminder
- `mika tasks cancel` / `mika tasks promote-deferred` — task mutations
- `mika mcp add/remove/enable/disable` — MCP mutations
- `mika agents create/delete/switch/clone/reset` — agent mutations
- `mika teams create/delete` — team mutations
- `mika skills install/uninstall/update/enable/disable/llm` — skill mutations
- `mika setup`, `mika chat`, `mika doctor` — interactive/specialized
- `mika sessions` — does not exist as a CLI command

## Implementation Steps

### Step 1: Add `Yaml` variant to `OutputFormat` enum

**File:** `crates/mika-cli/src/cli.rs:256-263`

Add `Yaml` variant to the existing enum:

```rust
#[derive(Clone, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Yaml,
}
```

**Dependency:** Add `serde_yaml.workspace = true` to `crates/mika-cli/Cargo.toml`. Already a workspace dep in root `Cargo.toml`.

### Step 2: Add a shared YAML serialization helper

**File:** `crates/mika-cli/src/commands/format_helper.rs` (new, minimal)

A tiny helper to avoid repeating `serde_yaml::to_string` + `serde_json::to_string_pretty` across every command. One function:

```rust
use crate::cli::OutputFormat;

pub fn print_formatted<T: serde::Serialize>(format: &OutputFormat, value: &T) -> anyhow::Result<()> {
    match format {
        OutputFormat::Text => {} // caller handles text
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(value)?),
    }
    Ok(())
}
```

Register in `crates/mika-cli/src/commands/mod.rs`.

### Step 3: Add `--format` to `MemoryArgs` (top-level)

**File:** `crates/mika-cli/src/cli.rs`

Add a `format` field to `MemoryArgs` struct and remove the per-subcommand `format` from `MemoryCommand::Search` (lift it to the parent):

```rust
pub struct MemoryArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
    #[command(subcommand)]
    pub command: Option<MemoryCommand>,
}
```

Remove the `format` field from `MemoryCommand::Search` — it now uses the parent's `format`.

### Step 4: Implement YAML + JSON output for memory subcommands

**File:** `crates/mika-cli/src/commands/memory.rs`

Thread `args.format` through all match arms. For each subcommand:

- **`None` (bare `mika memory`):** Build a serializable struct for core memory entries, match on format.
- **`People`:** `Person` already derives `serde::Serialize`. Match on format, call `print_formatted` for JSON/YAML.
- **`Commitments`:** `Commitment` already derives `serde::Serialize`. Same pattern.
- **`Preferences`:** `Preference` already derives `serde::Serialize`. Same pattern.
- **`Events`:** `Event` already derives `serde::Serialize`. Same pattern.
- **`Search`:** Update to use `args.format` instead of the removed per-variant format.

### Step 5: Add `--format` to `McpCommand::List`

**File:** `crates/mika-cli/src/cli.rs`

Add format to the `McpCommand::List` variant:

```rust
List {
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
},
```

**File:** `crates/mika-cli/src/commands/mcp.rs`

Thread format through `list_servers`. For JSON/YAML, serialize the server configs as structured data (server name, transport type, status, headers keys).

`McpServerConfig` already derives `Serialize` (it's serialized to JSON for `mcp.json`), but we'll want a purpose-built display struct that redacts header values and flattens the transport info.

### Step 6: Add `Yaml` arm to all existing `--format` match sites

Every existing command that matches on `OutputFormat` currently has two arms (`Text` and `Json`). Add the `Yaml` arm everywhere. The pattern is mechanical:

```rust
OutputFormat::Yaml => println!("{}", serde_yaml::to_string(&value)?),
```

**Files to update (all in `crates/mika-cli/src/commands/`):**
- `agents.rs` — `agents list`, `agents validate`
- `ask.rs` — `mika ask`
- `config.rs` — `config list`
- `kg.rs` — `kg status`, `kg list-agents`, `kg purge`, `kg validate`
- `logs.rs` — `mika logs`
- `model.rs` — `mika model`
- `provider.rs` — `mika provider`
- `reminders.rs` — `reminders list`, `reminders get`
- `skills.rs` — `skills list`, `skills validate`
- `skills_variants.rs` — `skills variants *`
- `status.rs` — `mika status`
- `tasks.rs` — `tasks list`, `tasks get`
- `teams.rs` — `teams list`, `teams status`, `teams log`, `teams validate`
- `webhook.rs` — `webhook list-dead`, `webhook replay`, `webhook replay-all`

### Step 7: Tests

Add tests to verify YAML output for at least:
- The new `Yaml` variant round-trips correctly (unit test on `OutputFormat`)
- Memory subcommands produce valid YAML (integration-style tests similar to existing JSON tests in `reminders.rs:139-230`)
- MCP list produces valid YAML

Existing JSON tests in `reminders.rs` and `tasks.rs` provide the pattern to follow.

### Step 8: Update CLI CLAUDE.md

**File:** `crates/mika-cli/CLAUDE.md`

Update `--format text|json` references to `--format text|json|yaml` throughout. Update the "Other `--format text|json` Commands" section header.

## Scope Boundaries

**In scope:**
- Add `Yaml` variant to `OutputFormat`
- Add `serde_yaml` dependency to `mika-cli`
- Add `--format` to memory subcommands (bare, people, commitments, preferences, events) and `mcp list`
- Add `Yaml` arm to all existing format match sites
- Tests for new functionality
- CLAUDE.md update

**Out of scope:**
- Adding `--format` to mutation commands (cancel, reset, create, delete, etc.)
- Adding a `sessions` CLI command (doesn't exist yet — separate feature)
- `mika doctor` already uses `--json` as a standalone flag — not migrating to `--format` (different UX contract)
- Shared formatting infrastructure beyond the minimal helper — each command owns its text rendering

## Risk Assessment

**Low risk.** This is additive-only:
- New enum variant is backward-compatible (existing `text`/`json` values unchanged)
- All DB types (`Person`, `Commitment`, `Preference`, `Event`, `CoreMemoryEntry`) already derive `serde::Serialize`
- `serde_yaml` is already a workspace dependency (used in `mika-agent` calibration)
- No behavior changes for users who don't pass `--format yaml`
- `mcp.json` config is already JSON-serialized — structured output is straightforward

## Estimated Complexity

~15 files touched, mostly mechanical (adding `Yaml` match arm). The memory and MCP changes are the only non-trivial work. Small-to-medium feature.
