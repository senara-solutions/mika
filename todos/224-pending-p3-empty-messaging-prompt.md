---
status: pending
priority: p3
issue_id: "224"
tags: [code-review, quality, skills-system]
dependencies: []
---

# Empty system_prompt.md for Messaging Skill

## Problem Statement
The messaging skill's `system_prompt.md` template is empty, meaning no instructions are injected for `send_message`. While the tool is self-documenting via its schema, having instructions would improve Claude's usage of the tool.

## Findings
- Location: `templates/skills/messaging/system_prompt.md` — empty file
- The `send_message` tool had instructions in prompt.rs before extraction (line ~147-148): guidance about when to proactively message vs wait
- These instructions were lost during the extraction to skills

## Proposed Solutions

### Option 1: Add messaging instructions from original prompt.rs
- **Pros**: Restores lost guidance
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `templates/skills/messaging/system_prompt.md`

## Acceptance Criteria
- [ ] Messaging skill has meaningful instructions

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — code-simplicity-reviewer agent
