---
status: ready
priority: p3
issue_id: "129"
tags: [code-review, architecture]
dependencies: []
---

# Core Memory Seeding Logic Duplicated Between CLI and Server

## Problem Statement

The core memory seeding logic (check if empty, read user.md, filter header, call `seed_core_memory`) is duplicated between `cli.rs` and `server/mod.rs`. Changes to seeding logic must be made in two places.

## Findings

- **Source:** architecture-strategist
- **Location:** `crates/mika-agent/src/cli.rs` and `crates/mika-agent/src/server/mod.rs:34-42`
- **Evidence:** Nearly identical code blocks in both files

## Proposed Solutions

### Option 1: Extract to a shared helper function
- **Pros**: Single source of truth, DRY
- **Cons**: Needs a home (lib.rs or a startup module)
- **Effort**: Small
- **Risk**: Low

## Technical Details

- **Affected Files**: `crates/mika-agent/src/cli.rs`, `crates/mika-agent/src/server/mod.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Core memory seeding logic exists in one place
- [ ] Both CLI and server use the shared function
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** architecture-strategist

## Resources

- PR #5: Phase 2 Container HTTP Server
