---
status: pending
priority: p2
issue_id: "654"
tags: [code-review, quality]
dependencies: []
---

# `run_gh`: Reuse `scrub_mika_env_vars` instead of inline loop

## Problem Statement

The MIKA_* env scrubbing loop in `run_gh` (lines 299-304) is copy-pasted from `executor.rs:24-29`. The same pattern also appears in `skills/git.rs:124-127`. Three copies of security-critical code increase the risk of divergence if the scrubbing strategy changes.

## Findings

- **Performance oracle**: Recommended making `scrub_mika_env_vars` from executor.rs `pub(crate)`.
- **Architecture reviewer**: Flagged as DRY violation across 3 locations.
- **Security sentinel**: Noted 3 copies of security-critical logic.

## Proposed Solutions

### Solution 1: Make executor function public and reuse (Recommended)
Make `executor::scrub_mika_env_vars` `pub(crate)` and call it from `run_gh` and `git.rs`.
- **Pros**: Single source of truth, any future scrubbing changes apply everywhere
- **Cons**: Minor cross-module dependency
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/executor.rs` (make pub), `crates/mika-agent/src/skills/builtin_handlers.rs`, `crates/mika-agent/src/skills/git.rs`
- **Components**: Environment scrubbing, subprocess spawning

## Acceptance Criteria

- [ ] Single `scrub_mika_env_vars` function used by all 3 call sites
- [ ] No inline MIKA_* env scrubbing loops remain

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Flagged by 3 reviewers |

## Resources

- `executor.rs:24-29` — canonical `scrub_mika_env_vars` implementation
