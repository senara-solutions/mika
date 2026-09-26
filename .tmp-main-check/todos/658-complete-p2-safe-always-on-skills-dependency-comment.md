---
status: complete
priority: p2
issue_id: "658"
tags:
  - code-review
  - security
  - skills
dependencies: []
---

# Document that safe_always_on_skills() intentionally excludes dependency resolution

## Problem Statement

`safe_always_on_skills()` filters skills for silent/heartbeat mode but does not resolve dependencies. If an always_on skill declares `dependencies = ["tmux"]`, tmux will NOT be loaded in silent mode. This is correct behavior (exec handlers must not run autonomously), but it's undocumented — a future change could accidentally route dependency resolution into the safe path.

## Findings

- **Source**: security-sentinel, pattern-recognition-specialist, agent-native-reviewer, learnings-researcher
- **Evidence**: `safe_always_on_skills()` at `mod.rs:127-143` iterates skills directly without calling `match_skills()`. No comment explains why.
- **Impact**: Future maintainer could unify matching logic and bypass the exec/http handler filter

## Proposed Solutions

### Option A: Add code comment + regression test
- Add a doc comment on `safe_always_on_skills()` explaining dependency resolution is intentionally excluded
- Add a test verifying that a safe always-on skill with a dependency on an exec-handler skill does NOT pull in the exec-handler skill
- **Pros**: Documents the security boundary, prevents regression
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/mod.rs`

## Acceptance Criteria

- [ ] `safe_always_on_skills()` has doc comment explaining no dependency resolution by design
- [ ] Test proves exec-handler dependency is NOT loaded in safe path

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-13 | Created from code review of PR #134 | Multiple agents flagged this gap independently |

## Resources

- PR: #134
- Known pattern: `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md`
