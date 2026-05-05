---
title: "CLI verb orthogonality: consistent subcommand surfaces across noun groups"
module: mika-cli
date: 2026-05-06
problem_type: best_practice
component: tooling
severity: medium
tags:
  - cli
  - clap
  - subcommands
  - agent-readiness
  - discoverability
applies_when:
  - Adding a new noun-level subcommand group (e.g., `mika sessions`)
  - Reviewing existing subcommand surfaces for agent compatibility
  - Designing CLI interfaces consumed by autonomous agents
---

# CLI Verb Orthogonality: Consistent Subcommand Surfaces

## Context

Autonomous agents discover CLI capabilities via `--help` parsing. When noun-level
subcommand groups (e.g., `mika tasks`, `mika agents`, `mika skills`) have inconsistent
verb surfaces, agents must special-case each group. The KISS principle dictates: same noun
shape, same verbs.

Issue #981 identified that `mika tasks` and `mika reminders` lacked explicit `list` and
`get` verbs that sibling surfaces (`mika agents`, `mika skills`) already had. Bare
invocation worked but was not discoverable via `--help` as a named subcommand.

## Guidance

Every noun-level subcommand group should expose a minimum orthogonal verb set:

| Verb | Purpose | Pattern |
|------|---------|---------|
| `list` | Enumerate resources | Always include `--format text\|json` |
| `get <id>` or `info <name>` | Detail view of a single resource | Use `get` for ID-based, `info` for name-based |
| Destructive verb (`cancel`, `delete`, `uninstall`) | Remove or stop a resource | Keep existing semantics |

### Implementation pattern (clap)

Use `Option<Command>` with `None` as a backward-compatible alias for `list`:

```rust
#[derive(Subcommand)]
pub enum TaskCommand {
    /// List active tasks
    List {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Show details for a specific task
    Get {
        id: String,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Cancel a task by ID
    Cancel { id: String },
}
```

In the handler, match `None` and `Some(List { .. })` together:

```rust
match args.command {
    None | Some(TaskCommand::List { .. }) => {
        let format = match &args.command {
            Some(TaskCommand::List { format }) => format.clone(),
            _ => OutputFormat::Text,
        };
        // list logic...
    }
    Some(TaskCommand::Get { id, format }) => { /* detail logic */ }
    Some(TaskCommand::Cancel { id }) => { /* cancel logic */ }
}
```

### UTF-8 safety in detail views

When truncating text fields for display, never use raw byte slices (`&v[..200]`).
Use char-boundary-safe truncation:

```rust
let display = match v.char_indices().nth(200) {
    Some((i, _)) => &v[..i],
    None => v,
};
```

The CI `byte-slice-lint` job (`scripts/check-byte-slices.sh`) enforces this.

## Why This Matters

- **Agent discoverability:** Agents parsing `--help` see explicit verbs and can construct
  commands without special-casing each noun group
- **Human muscle memory:** Users who learn `mika agents list` expect `mika tasks list`
- **Scripting consistency:** `--format json` on both `list` and `get` enables uniform
  pipeline integration
- **Backward compatibility:** `Option<Command>` with `None` alias preserves existing
  bare-invocation behavior

## Examples

Before (inconsistent):
```
$ mika tasks --help
Commands:
  cancel  Cancel a task by ID
  help    Print this message...

$ mika tasks list
error: unrecognized subcommand 'list'
```

After (orthogonal):
```
$ mika tasks --help
Commands:
  list    List active tasks
  get     Show details for a specific task
  cancel  Cancel a task by ID
  help    Print this message...

$ mika tasks list --format json
[{"id":"abc123...","label":"...","status":"pending",...}]
```
