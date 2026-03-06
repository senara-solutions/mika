---
status: pending
priority: p3
issue_id: "547"
tags: [code-review, quality, dry]
dependencies: []
---

# Extract default_self_model helper to eliminate duplication

## Problem Statement

The format string `"I am {display_name}. No interaction history yet."` is constructed identically in three places. If the format ever changes, all three must be updated in lockstep.

## Findings

- **Source:** Code simplicity + architecture review agents
- **Locations:**
  1. `crates/mika-agent/src/db.rs` `seed_core_memory` (line ~1466)
  2. `crates/mika-agent/src/tools/update_core_memory.rs` reset action (line ~169)
  3. `crates/mika-cli/src/commands/memory.rs` reset subcommand (line ~184)

## Proposed Solutions

### Option A: Add `fn default_self_model(display_name: &str) -> String` in db.rs
- **Approach:** Single helper function, three call sites simplified
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Single source of truth for the self_model default format
- [ ] All three call sites use the helper

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | DRY violation across 3 files |
