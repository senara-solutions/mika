---
status: pending
priority: p2
issue_id: 663
tags: [code-review, quality, skill]
---

# Expand google-workspace system prompt with missing operations

## Problem Statement

The system prompt documents read and create operations well but omits update, delete, reply-to-email, and Drive upload. The agent cannot discover these operations from the prompt alone. Also missing: exit code parsing guidance, retry hint for cold starts, and tool schema service enum hint.

## Findings

- **File**: `crates/mika-agent/templates/skills/google-workspace/system_prompt.md`
- **Agent**: Agent-native reviewer
- **Coverage**: 12/16 common operations documented

## Proposed Solutions

Add examples for:
- Reply to email (gmail messages send with threadId)
- Delete/trash a message
- Update a calendar event (calendar events patch)
- Delete a calendar event
- Upload file to Drive
- Add exit code parsing hint: "output starts with `Exit code: N`"
- Add retry hint for first-call timeout
- Update tools.json description to list valid services inline

Also update tool schema description in tools.json to: "First element must be a service: 'gmail', 'calendar', or 'drive'."

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] System prompt covers update/delete/reply operations
- [ ] Exit code parsing guidance added
- [ ] tools.json description mentions allowed services
