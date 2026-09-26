---
status: complete
priority: p2
issue_id: 736
tags: [code-review, documentation, architecture]
dependencies: []
---

# Update task-health solution doc to reflect callback injection (#314)

## Problem Statement

`docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md` prevention rule #2 explicitly states: "Gate heartbeat-specific data to heartbeat triggers only -- don't inject directive instructions into callback/reflection/skill_run prompts where the agent has a different job."

Issue #314 intentionally expands the guard to include callback triggers. The solution doc must be updated to reflect this architectural decision and document the rationale.

## Findings

- The solution doc was written when the guard was heartbeat-only
- The expansion to callbacks is intentional (issue #314) — callbacks need work item context for result correlation
- Reflection and SkillRun remain excluded (unchanged)

## Proposed Solutions

### Option A: Update prevention rule #2 (Recommended)
- Update the wording to reflect that callbacks now also receive task health
- Document the rationale: callback turns need work item awareness for result correlation
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **File:** `docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md`

## Acceptance Criteria

- [ ] Prevention rule #2 updated to include callback triggers
- [ ] Rationale documented for the expansion

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-29 | Created from code review of #314 | Solution doc contradicts new guard |

## Resources

- PR: #314
- Issue: #314
