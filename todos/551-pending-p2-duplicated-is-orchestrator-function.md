---
status: pending
priority: p2
issue_id: "551"
tags: [code-review, architecture, dry]
dependencies: []
---

# Duplicated `is_orchestrator()` function in delegate_task.rs and run_team.rs

## Problem Statement

The `is_orchestrator()` function is copy-pasted identically (13 lines) in two files. If one copy is updated and the other forgotten, authorization checks will silently diverge.

## Findings

- **Source:** Multiple review agents (security, architecture, simplicity, performance)
- **Locations:**
  - `crates/mika-agent/src/tools/delegate_task.rs:15-27`
  - `crates/mika-agent/src/tools/run_team.rs:21-33`
- **Evidence:** Both implementations are byte-for-byte identical

## Proposed Solutions

### Solution A: Extract to `crate::tools` mod.rs (Recommended)

Add `pub(crate) fn is_orchestrator(home_dir: &Path, agent_id: &str) -> bool` to `crates/mika-agent/src/tools/mod.rs`. Import from both tool files.

- **Pros:** Minimal change, keeps it crate-local, both files already import from `super`
- **Cons:** None
- **Effort:** Small
- **Risk:** None

### Solution B: Extract to `mika_common::agent` or `mika_common::team`

- **Pros:** Available across crates if needed in the future
- **Cons:** Slightly broader visibility than needed right now
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Single `is_orchestrator()` function in a shared location
- [ ] Both `delegate_task.rs` and `run_team.rs` import and use the shared function
- [ ] All existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Identified by all review agents |
