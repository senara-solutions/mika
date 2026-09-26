---
title: "Rename CLI ID flags for naming consistency"
type: refactor
status: completed
date: 2026-03-17
---

# Rename CLI ID flags for naming consistency

## Overview

The Mika CLI has an inconsistent flag naming pattern. Flags that accept opaque identifiers (UUIDs, session IDs) should follow the `--{noun}-id` convention, but two flags deviate:

| Flag | Accepts | Should be |
|------|---------|-----------|
| `--session` | Session ID (UUID/string) | `--session-id` |
| `--parent-task` | Task UUID | `--parent-task-id` |

Meanwhile, `--task-id` and `--run-id` already follow the correct pattern. Flags that accept human-readable names (`--agent`, `--team`) correctly omit `-id`.

Pre-1.0, so breaking changes are shipped without backward compatibility (per CLAUDE.md).

## Acceptance Criteria

- [x] `--session` renamed to `--session-id` across CLI
- [x] `--parent-task` renamed to `--parent-task-id` across CLI
- [x] All Rust usage sites updated (struct fields, function params, error messages, comments)
- [x] CLAUDE.md updated
- [x] Active docs updated (`docs/solutions/`, `todos/`)
- [x] `cargo test` passes
- [x] `cargo clippy` passes

## MVP

### 1. `crates/mika-cli/src/cli.rs` — Rename struct fields

```rust
// Cli struct (line 17): session → session_id
pub session_id: Option<String>,

// AskArgs struct (line 135-137): parent_task → parent_task_id
pub parent_task_id: Option<String>,
```

Clap auto-derives `--session-id` and `--parent-task-id` from field names.

### 2. `crates/mika-cli/src/main.rs` — Update field access

```rust
// Lines 165, 167, 181: cli.session → cli.session_id
cli.session_id.as_deref()

// Line 182: args.parent_task → args.parent_task_id
args.parent_task_id.as_deref()
```

### 3. `crates/mika-cli/src/commands/ask.rs` — Update param names, errors, comments

```rust
// Line 24: session → session_id parameter
// Line 25: parent_task → parent_task_id parameter
// Line 31: comment "When --session is passed" → "When --session-id is passed"
// Line 36: error "--session value" → "--session-id value"
// Line 38: let session_id = session → let session_id = session_id (already named correctly downstream)
// Line 147: comment "--parent-task" → "--parent-task-id"
```

### 4. `crates/mika-cli/src/commands/chat.rs` — Update param name, error

```rust
// Line 49: session parameter in spawn_agent_worker
// Line 63: error "--session value" → "--session-id value"
// Line 65: session → session_id
```

### 5. `CLAUDE.md` — Update flag documentation

Update the `mika ask` description (line ~29) to reference `--session-id` and `--parent-task-id`.

### 6. Active docs — Update references

- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — any `--session` references
- `todos/626-pending-p3-parent-task-cli-yagni.md` — `--parent-task` references

### 7. Historical docs — Leave unchanged

CHANGELOG.md, plan docs, brainstorm docs are historical records and should not be updated.

## Notes

- `--session-id` remains `global = true` — scoping is out of scope (tracked separately)
- No hidden aliases needed — no external consumers exist for either flag
- The `cli-reference.md` auto-regenerates at startup via clap-markdown
- Error messages in `ask.rs:36` and `chat.rs:63` must be updated to avoid referencing a non-existent flag
