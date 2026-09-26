---
status: complete
priority: p2
issue_id: "398"
tags: [code-review, performance, marketplace, pr-56]
dependencies: []
---

# update_skill reads lock file twice (TOCTOU)

## Problem Statement

`update_skill()` reads the lock file at line 150 (to get the entry) and again at line 193 (to update it). The second read is unnecessary — the first lock can be made mutable and reused. The double-read also creates a TOCTOU window where concurrent modifications could be silently lost.

## Findings

- **Source**: performance-oracle, architecture-strategist, code-simplicity-reviewer
- **File**: `crates/mika-agent/src/skills/install.rs:150,193`

## Proposed Solutions

### Option A: Reuse first lock read (Recommended)

Change `let lock = read_lock(...)` at line 150 to `let mut lock = read_lock(...)` and remove the second `read_lock()` at line 193.

- Effort: Small (2-line change)
- Risk: Low

## Acceptance Criteria

- [ ] Lock file read once per `update_skill` call
- [ ] Tests pass

## Resources

- `crates/mika-agent/src/skills/install.rs:150,193`
