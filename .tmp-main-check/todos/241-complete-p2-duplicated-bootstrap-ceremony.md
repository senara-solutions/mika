---
status: complete
priority: p2
issue_id: "241"
tags: [code-review, duplication, architecture]
dependencies: []
---

# Duplicated bootstrap ceremony across mika-spirit.rs and setup.rs

## Problem Statement

The 5-step bootstrap sequence (create agents/ dir, bootstrap_agent, write_active_agent, write_default_if_missing for config.toml) is copy-pasted between `mika-spirit.rs` (lines 12-21) and `setup.rs` (lines 16-25). If the bootstrap sequence changes, both must be updated.

## Findings

- **Source:** Pattern Recognition Specialist
- **Files:** `crates/mika-agent/src/bin/mika-spirit.rs:12-21`, `crates/mika-cli/src/commands/setup.rs:16-25`

## Proposed Solutions

### Option A: Extract to `home::bootstrap_fresh_install` [Recommended]
Create `pub fn bootstrap_fresh_install(home_dir: &Path) -> Result<()>` in `mika-common/src/home.rs` that encapsulates the full sequence.

- **Pros:** Single source of truth, DRY
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Single bootstrap function in `home.rs`
- [ ] Both `mika-spirit.rs` and `setup.rs` call the shared function

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Shotgun Surgery anti-pattern |
