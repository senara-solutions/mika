---
title: CLI flag naming convention — ID suffix for opaque identifiers
category: architecture-patterns
date: 2026-03-17
severity: low
module: mika-cli
tags: [cli, clap, naming-convention, consistency]
---

# CLI flag naming — `--{noun}-id` for opaque identifiers

## Problem

CLI flags that accept opaque identifiers (UUIDs, session IDs) were inconsistently named. `--task-id` and `--run-id` used the `-id` suffix but `--session` and `--parent-task` did not, despite also accepting IDs.

## Convention

| Flag accepts | Pattern | Examples |
|-------------|---------|----------|
| Opaque ID (UUID, generated string) | `--{noun}-id` | `--task-id`, `--run-id`, `--session-id`, `--parent-task-id` |
| Human-readable name | `--{noun}` | `--agent`, `--team` |

## Implementation Note: Parameter Shadowing

When a clap struct field and its downstream local variable share the same name (e.g., both called `session_id`), the common pattern of shadowing the `Option<&str>` parameter with the resolved `String` value breaks the "was a value explicitly provided?" check:

```rust
// Before rename: distinct names, no issue
fn run(session: Option<&str>) {
    let session_id = session.unwrap_or_else(|| generate());
    if session.is_some() { /* check original */ }
}

// After rename: shadowing kills the original binding
fn run(session_id: Option<&str>) {
    let reusing_session = session_id.is_some(); // save before shadow
    let session_id = session_id.unwrap_or_else(|| generate());
    if reusing_session { /* use saved flag */ }
}
```

## Prevention

When renaming clap fields, check if the parameter name will collide with a downstream local variable. If so, extract a boolean flag before the shadowing line.
