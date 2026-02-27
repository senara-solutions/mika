---
status: complete
priority: p3
issue_id: "319"
tags: [code-review, cleanup, dead-code]
dependencies: []
---

# Dead search.sh File After Builtin Handler Migration

## Problem Statement

`templates/skills/web-search/handlers/search.sh` is no longer used after the web_search skill was migrated from an exec handler to a builtin handler. The tools.json was updated to `"handler_type": "builtin"` but the shell script was not removed.

## Findings

**Source:** code-simplicity-reviewer, pattern-recognition-specialist

**Location:** `templates/skills/web-search/handlers/search.sh`

The tools.json diff shows handler_type changed from "exec" to "builtin", making the shell handler dead code.

## Proposed Solutions

### Option A: Delete search.sh
- Remove the dead file
- **Pros:** Clean codebase
- **Cons:** None
- **Effort:** Trivial
- **Risk:** None

## Technical Details

- **Affected files:** `templates/skills/web-search/handlers/search.sh`

## Acceptance Criteria

- [ ] search.sh is deleted
- [ ] No references to search.sh remain in the codebase

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #28 code review | |

## Resources

- PR: #28
