---
module: task-engine
date: 2026-05-08
problem_type: best_practice
component: tooling
severity: low
tags:
  - task-tools
  - process-id
  - query-parity
  - proactive-state-checking
applies_when:
  - A DB column is written by one subsystem but not surfaced in any agent-facing query tool
  - An agent needs to introspect runtime state (e.g., PIDs, session IDs) via the DB
  - A new write path is added without a corresponding read path in the tool layer
---

# Expose internal state in query tools when the DB column already exists

## Context

The `tasks.process_id` column was added to track subprocess PIDs at spawn time and clear them on exit. The callback watchdog (`check_callback_process_liveness`) already read this column for liveness detection. However, neither `check_task` nor `list_tasks` surfaced `process_id` in their output, so the agent had no query path to answer "what is the PID of the running session for task N?" — a gap that blocked cancel-by-PID logic in a sibling ticket.

This is a recurring pattern: a column exists, internal subsystems use it, but agent-facing tools don't expose it. The CLAUDE.md convention ("new write tools should have a corresponding query tool") applies equally to new columns on existing tools.

## Guidance

When a DB column is populated by one code path (spawn, callback, migration) but not surfaced in the agent's query tools, close the gap by adding conditional display in the existing read tools — don't wait for a separate ticket.

**For verbose detail tools** (`check_task` pattern): use `if let Some(val) = field { writeln!(output, "Label: {val}"); }` — the same conditional-writeln pattern used for Source, Reference, Completed, and Metadata.

**For compact list tools** (`list_tasks` pattern): use `.map(|v| format!(" key:{v}")).unwrap_or_default()` appended to the format string — the same annotation pattern used for `ref:`, `src:`, `type:`, `children:`.

## Why This Matters

Without query parity, the agent cannot introspect state that the engine already tracks. This creates a class of questions ("what PID is running?", "which session owns this task?") that are answerable only via raw DB queries — breaking the self-service model. The proactive state checking convention exists precisely to prevent this gap from accumulating silently.

## When to Apply

- After adding a column that any internal subsystem reads or writes
- When reviewing a PR that adds `set_*` or `clear_*` methods without corresponding display changes
- When an agent asks a question that should be answerable from the DB but isn't

## Examples

**check_task (verbose):**

```rust
// Conditional display — only when set (most tasks have process_id = None)
if let Some(pid) = task.process_id {
    writeln!(output, "Process ID: {pid}").unwrap();
}
```

**list_tasks (compact annotation):**

```rust
// Compact annotation — rarest fields go last to keep common case clean
let pid = task
    .process_id
    .map(|p| format!(" pid:{p}"))
    .unwrap_or_default();

// Append to format string after existing annotations
format!("...{children}{pid})")
```

Key design choice: `process_id` is `Option<i64>` (a `Copy` type), so use `if let Some(pid)` not `if let Some(ref pid)`, and `.map(|p| ...)` works directly without `.as_deref()`.
