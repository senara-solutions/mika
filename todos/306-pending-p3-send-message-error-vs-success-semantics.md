---
status: pending
priority: p3
issue_id: 306
tags: [code-review, agent-native, quality]
dependencies: []
---

# Consider ToolOutput::error for send_message no-sender path

## Problem Statement

`send_message` returns `ToolOutput::success(...)` when no sender is configured and the message was NOT delivered. The structured `is_error: false` flag tells Claude the tool call succeeded, while the text content says "NOT delivered." This creates conflicting signals.

## Findings

- **Agent-native review**: Other tools in the codebase (empty text, too-long text) correctly use `ToolOutput::error` for failure conditions
- **Simplicity review**: Notes this is a deliberate design choice — returning success prevents retry loops
- **Counter-argument**: The tool DID succeed at its local operation (saving to DB via `save_message`), but failed at external delivery. `success` with warning text is defensible.

## Proposed Solutions

### Solution A: Change to ToolOutput::error
- Aligns structured signal with content
- Pros: Unambiguous failure signal; matches other tool patterns
- Cons: Claude may retry in a loop; may generate error-handling text
- Effort: Small
- Risk: Medium (behavioral change in agent responses)

### Solution B: Keep success with documented rationale
- Add a code comment explaining the intentional choice
- Pros: No behavioral change; prevents retry loops
- Cons: Semantic mismatch remains
- Effort: Small
- Risk: Low

## Recommended Action

(To be filled during triage)

## Technical Details

- **Affected file**: `crates/mika-agent/src/tools/send_message.rs:59`

## Acceptance Criteria

- [ ] Decision documented in code comment regardless of choice
- [ ] If changed to error: verify agent doesn't retry in a loop

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-27 | Created from PR #25 review | success vs error for partial-success is a design decision |

## Resources

- PR #25: https://github.com/senara-solutions/mika/pull/25
