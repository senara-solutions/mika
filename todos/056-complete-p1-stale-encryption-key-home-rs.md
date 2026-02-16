---
status: complete
priority: p1
issue_id: "056"
tags: [code-review, bug, config, rust-v2]
dependencies: []
---

# Stale MIKA_ENCRYPTION_KEY Reference in home.rs DEFAULT_CONFIG

## Problem Statement

The `DEFAULT_CONFIG` constant in `crates/mika-common/src/home.rs` (line ~85) still references `MIKA_ENCRYPTION_KEY` in the generated `.env` template. This env var was removed in the encryption strip refactor. Users following the generated config will encounter a stale/confusing reference to a key that no longer exists.

**Location:** `crates/mika-common/src/home.rs` — `DEFAULT_CONFIG` constant

**Reported by:** security-sentinel, architecture-strategist, code-simplicity-reviewer, learnings-researcher (4/6 agents)

## Findings

- `DEFAULT_CONFIG` is a string constant used to generate `~/.mika/.env` during first-run setup
- It still contains a line referencing `MIKA_ENCRYPTION_KEY`
- The `config.rs` `Settings` struct no longer has an `encryption_key` field
- `.env.example` was already updated to remove this reference, but `home.rs` was missed

## Proposed Solutions

### Option A: Remove the MIKA_ENCRYPTION_KEY line from DEFAULT_CONFIG (Recommended)
Simply delete the line from the constant.
- **Pros:** Consistent with the rest of the codebase
- **Cons:** None
- **Effort:** Tiny (1 line)
- **Risk:** None

## Acceptance Criteria

- [ ] `DEFAULT_CONFIG` in home.rs does not reference `MIKA_ENCRYPTION_KEY`
- [ ] `cargo test` passes
- [ ] Generated `.env` file only contains valid config keys

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | Missed in commit eb03ea7 — home.rs was not in the plan's file list |
| 2026-02-24 | Fixed — removed MIKA_ENCRYPTION_KEY line from DEFAULT_CONFIG | 1-line fix |
