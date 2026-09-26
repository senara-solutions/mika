---
status: complete
priority: p1
issue_id: "201"
tags: [code-review, agent-native, regression]
dependencies: []
---

# `/reset <block>` Command Dropped With No Replacement

## Problem Statement

The old CLI's `/reset <block>` slash command was the only way to reset a corrupted or unwanted core memory block back to its default/seed value. It was deleted along with `cli.rs` and has no replacement in the new `mika-cli` crate. This is a feature regression that removes a data recovery/debugging capability.

## Findings

- **Source:** agent-native-reviewer (Critical-1), architecture-strategist (9a), pattern-recognition-specialist (7a)
- **Location:** Deleted `crates/mika-agent/src/cli.rs:154-181`. Not present in `crates/mika-cli/src/commands/memory.rs`.
- **Evidence:** The old code handled special logic for `user_summary` (re-reading `user.md`), validated block names against `CORE_MEMORY_SECTIONS`, and called `db.set_core_memory()`. None of this exists in the new crate.
- **Impact:** Users cannot reset corrupted core memory blocks. No workaround except direct SQLite manipulation.

## Proposed Solutions

### Option 1: Add `mika memory reset <block>` subcommand
- **Pros**: Preserves feature parity, fits naturally in the existing clap structure
- **Cons**: None
- **Effort**: Small (port ~30 lines from deleted cli.rs)
- **Risk**: Low

Add to `cli.rs`:
```rust
/// Reset a core memory block to its default value
Reset { block: String },
```

Port the handler logic from the deleted code, using `CORE_MEMORY_SECTIONS` for validation and special `user_summary` handling.

### Option 2: Skip — let users reset via the agent
- **Pros**: No code to write
- **Cons**: Agent cannot reset its own memory blocks to defaults; this was an admin/debug feature intentionally outside the agent loop
- **Effort**: None
- **Risk**: Medium (loses debug capability)

## Recommended Action

Option 1 — port the logic from the deleted cli.rs.

## Technical Details

- **Affected files:** `crates/mika-cli/src/cli.rs`, `crates/mika-cli/src/commands/memory.rs`
- **Imports needed:** `mika_agent::db::{CORE_MEMORY_SECTIONS, core_memory_section_names}`

## Acceptance Criteria

- [ ] `mika memory reset user_summary` resets to default (re-reads user.md if available)
- [ ] `mika memory reset persona` resets to default value
- [ ] `mika memory reset invalid_block` prints error with valid block names
- [ ] Matches behavior of deleted `/reset` command

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | Feature parity check caught this regression |

## Resources

- Commit: 399ebf0
- Deleted code: `crates/mika-agent/src/cli.rs:154-181`
