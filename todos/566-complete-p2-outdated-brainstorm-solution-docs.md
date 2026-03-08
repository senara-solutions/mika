---
status: complete
priority: p2
issue_id: 566
tags:
  - code-review
  - documentation
dependencies: []
---

# Brainstorm and solution docs describe prompt-only approach, but implementation adds DB constraints

## Problem Statement

The brainstorm doc (`docs/brainstorms/2026-03-08-proactive-state-checking-brainstorm.md`) says "Prompt-only solution chosen over code-level guards" and lists "DB-level uniqueness constraints for events table" as out-of-scope. The solution doc (`docs/solutions/logic-errors/agent-creates-duplicates-after-compaction.md`) also describes only the prompt approach.

The actual implementation adds DB UNIQUE partial indexes and tool-level constraint catching — the opposite of what the docs describe. The docs are now factually wrong.

## Findings

- **Flagged by:** Code Simplicity Reviewer, Learnings Researcher
- Brainstorm "Key Decisions" section #1: "Prompt-only implementation — no code-level duplicate detection"
- Brainstorm "Out of scope" section: "DB-level uniqueness constraints for events table"
- Solution doc describes only the prompt instruction

## Proposed Solutions

### Option A: Update docs to reflect actual implementation (Recommended)

Update both docs to describe the three-layer defense-in-depth approach: prompt instruction + DB constraints + tool-level catch.

- **Pros:** Accurate documentation
- **Cons:** Minor effort
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Brainstorm doc updated to describe the DB constraint approach as the final decision
- [ ] Solution doc updated to include DB indexes and constraint catching
- [ ] "Out of scope" section updated to remove items that are now in scope

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Found during code review | Docs drifted from implementation |
