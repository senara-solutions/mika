---
status: complete
priority: p3
issue_id: "660"
tags:
  - code-review
  - quality
  - skills
dependencies: []
---

# validate_dependencies uses to_lowercase instead of eq_ignore_ascii_case

## Problem Statement

`validate_dependencies()` builds a `HashSet<String>` of lowercased names and compares with `.to_lowercase()`. The rest of the module (`apply_overrides`, `match_skills`) uses `eq_ignore_ascii_case`. Minor style inconsistency + unnecessary string allocations.

## Proposed Solutions

### Option A: Refactor to use eq_ignore_ascii_case
- Replace HashSet lookup with linear scan using `eq_ignore_ascii_case`
- Consistent with rest of codebase, avoids lowercased string clones
- **Effort**: Small

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/mod.rs:84-102`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-13 | Created from code review of PR #134 | Pattern recognition specialist flagged |
