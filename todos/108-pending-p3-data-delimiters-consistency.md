---
status: pending
priority: p3
issue_id: "108"
tags: [code-review, agent-native, security]
dependencies: []
---

# Add XML data delimiters to core memory and commitments in prompt

## Problem Statement
The PR added XML data delimiters for conversation summary (`<context type="summary" trust="data">`) and reminder data (`<reminder-data>`), but core memory and commitments sections in the prompt lack similar delimiters. Inconsistent delimiter usage reduces prompt injection defense coverage.

## Findings
- File: `crates/mika-agent/src/prompt.rs` and `crates/mika-agent/src/agent.rs`
- Summary and reminders use XML delimiters ✓
- Core memory injected without delimiters ✗
- Commitments injected without delimiters ✗
- Flagged by: Agent-Native Reviewer (Warning)

## Proposed Solutions

### Option 1: Add consistent XML delimiters to all data sections (Recommended)
```
<core-memory>
{core memory content}
</core-memory>

<commitments>
{commitments content}
</commitments>
```
**Effort:** Small
**Risk:** Low — may require prompt test updates

## Technical Details
**Affected files:** `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria
- [ ] All user-data sections in prompt wrapped with XML delimiters
- [ ] Consistent delimiter style across all data types
- [ ] Tests updated

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Agent-Native Reviewer found inconsistent prompt injection defense
