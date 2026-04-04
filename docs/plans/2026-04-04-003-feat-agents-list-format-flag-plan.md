---
title: "feat: Add --format flag to mika agents list"
type: feat
status: completed
date: 2026-04-04
issue: "#370"
---

# Add --format flag to `mika agents list`

## Overview

Add a `--format text|json` flag to `mika agents list` for consistent machine-parseable output, reusing the existing `OutputFormat` enum already used by `mika ask` and `mika teams log`.

## Problem Statement

The current `mika agents list` output uses a header line (`Agents:`) and indented agent names with `(active)` markers. This is awkward to parse in scripts. The root `Makefile`'s `deploy-skills` target uses `tail -n +2 | awk '{print $1}'` to extract agent names — fragile and breaks on formatting changes.

## Proposed Solution

1. Convert `AgentsCommand::List` from a unit variant to a struct variant with a `format: OutputFormat` field
2. Branch the `list()` handler on format: text preserves current output, JSON emits a structured array
3. Update the root Makefile to use `--format json | jq -r '.[].name'`

## Implementation

### 1. Add `format` field to `AgentsCommand::List` (`crates/mika-cli/src/cli.rs`)

Convert the unit variant to a struct variant:

```rust
// Before
List,

// After
List {
    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
},
```

This scopes `--format` to `agents list` only — not `create`, `delete`, `switch`, or `clone`.

### 2. Update dispatch in `crates/mika-cli/src/commands/agents.rs`

Destructure the format field and pass to `list()`:

```rust
// Before
AgentsCommand::List => list(&global_home),

// After
AgentsCommand::List { format } => list(&global_home, &format),
```

### 3. Update `list()` handler (`crates/mika-cli/src/commands/agents.rs`)

Branch on format, following the `teams log` pattern:

```rust
fn list(global_home: &Path, format: &OutputFormat) -> Result<()> {
    let agents = agent::list_agents(global_home);
    let active = home::read_active_agent(global_home)?;

    match format {
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = agents
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "name": name,
                        "active": name == &active,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        OutputFormat::Text => {
            if agents.is_empty() {
                // preserve current empty-list behavior
            } else {
                // preserve current text output
            }
        }
    }
    Ok(())
}
```

**JSON schema:**
```json
[
  {"name": "mika", "active": true},
  {"name": "mika-dev", "active": false},
  {"name": "mika-qa", "active": false}
]
```

**Edge cases:**
- Empty list: emit `[]` (no hint text to stdout — keeps output valid JSON)
- Single agent: emit `[{"name": "mika", "active": true}]`

### 4. Update `agent_override()` match arm (`crates/mika-cli/src/cli.rs`)

The `Commands::Agents` match arm in `agent_override()` must be updated if the `List` variant destructuring changes. Since `List` doesn't provide an agent override, the match arm just needs to accommodate the new struct variant syntax:

```rust
// Update from:
AgentsCommand::List => None,
// To:
AgentsCommand::List { .. } => None,
```

### 5. Update root Makefile (`Makefile` in mika-platform)

```makefile
# Before
@for agent in $$(mika agents list | tail -n +2 | awk '{print $$1}'); do \

# After
@for agent in $$(mika agents list --format json | jq -r '.[].name'); do \
```

**Note:** `jq` is already a documented host dependency (installed in Docker images, expected on dev machines).

## Acceptance Criteria

- [x] `mika agents list --format json` outputs a JSON array of agent objects with `name` (string) and `active` (boolean) fields
- [x] `mika agents list` (no flag) preserves current human-friendly output exactly
- [x] `mika agents list --format text` produces same output as no flag
- [x] Empty agent list: JSON emits `[]`, text preserves current message
- [x] Uses existing `OutputFormat` enum — no new variants or types
- [ ] Root Makefile `deploy-skills` target updated to use `--format json` + `jq` (separate PR on mika-platform repo)
- [x] All existing tests pass (`cargo test`)
- [x] `cargo clippy` clean

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-cli/src/cli.rs` | Convert `AgentsCommand::List` to struct variant with `format` field; update `agent_override()` match |
| `crates/mika-cli/src/commands/agents.rs` | Update dispatch + `list()` to branch on format |
| `../Makefile` (mika-platform root) | Update `deploy-skills` to use `--format json \| jq` |

## Sources

- Issue: [#370](https://github.com/senara-solutions/mika/issues/370)
- Existing pattern: `AskArgs.format` at `crates/mika-cli/src/cli.rs:174`
- Existing pattern: `TeamsCommand::Log { format }` at `crates/mika-cli/src/cli.rs:286`
- Handler pattern: `commands/teams.rs` lines 188-226
- Institutional learning: `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`
