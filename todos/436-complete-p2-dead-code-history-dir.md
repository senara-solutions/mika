---
status: pending
priority: p2
issue_id: "436"
tags: [code-review, dead-code, cleanup]
dependencies: []
---

# Dead Code: history_dir() Function

## Problem Statement

The `history_dir()` function in `crates/mika-common/src/team.rs` and its test are dead code after the TOML-to-SQLite migration. All callers were removed when `teams/history.rs` was deleted. The `teams create` command no longer creates a `history/` directory.

## Findings

- Architecture strategist and pattern recognition specialist both flagged this
- Function exists at `crates/mika-common/src/team.rs` line ~151

## Proposed Solutions

### Option A: Remove function and test (Recommended)

Delete `history_dir()` and its test from `crates/mika-common/src/team.rs`.

- **Pros:** Removes dead code, reduces confusion
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `history_dir()` function removed
- [ ] Associated test removed
- [ ] `cargo test` passes
