---
status: pending
priority: p2
issue_id: "585"
tags: [code-review, architecture]
dependencies: []
---

# Duplicate DB Queries: list_agent_sessions_paginated vs list_sessions_paginated

## Problem Statement
`list_agent_sessions_paginated` is nearly identical to `list_sessions_paginated` with a hardcoded agent_id filter. Same for `count_agent_sessions` vs `count_sessions`. This adds ~70 lines of duplicated code.

## Findings
- **Source:** Code Simplicity Reviewer
- **Location:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`

## Proposed Solutions
Remove `list_agent_sessions_paginated` and `count_agent_sessions`. Use `list_sessions_paginated(Some(agent_id), None, limit, offset)` instead.

## Acceptance Criteria
- [ ] Duplicate functions removed
- [ ] Agent sessions handler uses the general paginated query

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Simplicity Reviewer found duplication |

## Resources
- PR #89
