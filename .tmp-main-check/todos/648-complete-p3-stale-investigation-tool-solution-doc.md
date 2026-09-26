---
status: complete
priority: p3
issue_id: 648
tags: [code-review, documentation]
dependencies: []
---

# Stale field names in investigation tool solution doc

## Problem Statement

The solution doc `docs/solutions/architecture-patterns/conditional-investigation-tool-registration.md`
contains code examples with the old `github_token` field name. Unlike the plan
doc (which is a historical record), this solution doc is a living reference that
developers consult for the current pattern.

## Findings

- Line 9: `github_token, github_repo Settings fields`
- Line 36: `github_token: Option<String>`
- Line 58: `config.github_token`

Detected by: pattern-recognition-specialist, agent-native-reviewer, architecture-strategist

## Proposed Solutions

### Option A: Update the solution doc
- Replace `github_token` with `investigate_github_token` in the code examples
- **Pros:** Accurate reference for current codebase
- **Cons:** Minor effort
- **Effort:** Small
- **Risk:** None

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `docs/solutions/architecture-patterns/conditional-investigation-tool-registration.md`

## Acceptance Criteria

- [x] Code examples in the solution doc reflect current field/env var names

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Solution docs are living references; plan docs are historical |
| 2026-03-12 | Fixed during doc audit | All 3 stale references updated in the solution doc |

## Resources

- Pattern recognition, agent-native, and architecture agents all flagged this
