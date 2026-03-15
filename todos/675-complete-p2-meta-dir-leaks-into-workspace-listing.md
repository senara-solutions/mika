---
status: complete
priority: p2
issue_id: "675"
tags: [code-review, architecture, information-leakage]
dependencies: []
---

# .meta directory files leak into workspace listing and reads

## Problem Statement

The `collect_files()` function in `list_workspace.rs` does not filter out the `.meta/` subdirectory. Engine-internal metadata files (`goal.md`, `assignments.md`, `critic_feedback.md`, `deliverable.md`) appear in workspace listings shown to agents. Similarly, `read_workspace` allows agents to read these files. Agents should not interact with engine-internal metadata.

## Findings

- **Pattern Recognition**: Medium severity. `.meta/` files are engine-internal orchestration artifacts. Exposing them to agents leaks orchestration context (critic feedback, assignments) which could influence agent behavior in unintended ways.
- **Code Simplicity**: Related — metadata files have no intentional consumer via workspace tools.

**Affected files:**
- `crates/mika-agent/src/tools/list_workspace.rs` (`collect_files` function)
- `crates/mika-agent/src/tools/read_workspace.rs` (`read_from_dir` method)

## Proposed Solutions

### Option A: Skip dotfiles/dotdirs in `collect_files` (Recommended)
Add a check to skip directory entries starting with `.` in the `collect_files` recursive walk.
- **Pros:** Simple, conventional (dotfiles are hidden by convention), future-proof
- **Cons:** Would also hide any `.`-prefixed agent files (unlikely in practice)
- **Effort:** Small
- **Risk:** None

### Option B: Specifically skip `.meta` directory
Check for `.meta` by name in `collect_files` and reject `.meta/` prefix in `read_from_dir`.
- **Pros:** Targeted, no side effects on other dotfiles
- **Cons:** Brittle if more internal dirs are added
- **Effort:** Small
- **Risk:** None

## Recommended Action

Option A — skip all dotfile/dotdir entries in `collect_files`.

## Acceptance Criteria

- [ ] `.meta/` files do not appear in `list_workspace` output
- [ ] `read_workspace` rejects paths starting with `.meta/`
- [ ] Test coverage for dotfile filtering

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Pattern recognition specialist flagged |

## Resources

- Pattern recognition finding #2b
