---
status: complete
priority: p3
issue_id: "208"
tags: [code-review, quality, simplification]
dependencies: []
---

# Dead Code and Minor Simplifications

## Problem Statement

Several small issues identified across the new crate: dead constant, always-true return value, no-op event variant, duplicate markdown heading branches.

## Findings

- **Source:** code-simplicity-reviewer (Findings 2a-2c, 5)
- **Locations:**
  1. `crates/mika-agent/src/db.rs:9` — `pub const SCHEMA_VERSION` is never imported (dead code). The `schema_version()` method is used instead.
  2. `crates/mika-cli/src/tui/input.rs:6` — `handle_key` returns `bool` but every path returns `true` and no caller reads it.
  3. `crates/mika-cli/src/tui/event.rs:11` — `AppEvent::Resize` is defined, dispatched, matched, and does nothing (comment says "Terminal handles resize automatically").
  4. `crates/mika-cli/src/tui/markdown.rs:25-43` — `# ` and `## ` heading branches produce identical styling (both Yellow+Bold). Should either differentiate or merge.

## Proposed Solutions

### Single cleanup pass
- Remove `pub const SCHEMA_VERSION` from db.rs
- Change `handle_key` return to `()`
- Remove `AppEvent::Resize` variant and its match arms
- Collapse duplicate heading branches in markdown.rs
- **Effort**: Small (~30 lines changed)
- **Risk**: Low

## Recommended Action

Single cleanup commit addressing all four items.

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-cli/src/tui/input.rs`, `crates/mika-cli/src/tui/event.rs`, `crates/mika-cli/src/commands/chat.rs`, `crates/mika-cli/src/tui/markdown.rs`

## Acceptance Criteria

- [ ] `cargo build` clean, no warnings
- [ ] No dead code in new crate
- [ ] Heading styles are intentionally differentiated or merged

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
