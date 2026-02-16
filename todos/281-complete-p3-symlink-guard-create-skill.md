---
status: complete
priority: p3
issue_id: 281
tags: [code-review, security, skills]
dependencies: []
---

# Add Symlink Guard to create_skill Tool

## Problem Statement

There is a TOCTOU gap between `skill_dir.exists()` check and `create_dir()`. In a compromised-container scenario, a symlink at `skills/{name}` could redirect writes to an arbitrary directory.

## Findings

- **Security sentinel**: "TOCTOU gap between exists() and create_dir()"
- Mitigated by: container isolation, strict name validation (no `/` or `..`), attacker needs write access to skills/
- Low practical risk given architecture

## Proposed Solutions

### Option A: Canonicalize and verify after creation
- After `create_dir`, canonicalize both `skills_dir` and `skill_dir`
- Verify canonical skill_dir starts with canonical skills_dir
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] After directory creation, canonicalize and verify containment
- [ ] Add test for symlink detection

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | TOCTOU race condition identified |
