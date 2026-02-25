---
status: pending
priority: p2
issue_id: "217"
tags: [code-review, architecture, skills-system]
dependencies: []
---

# Hardcoded Trigger Text for Silent Mode Skill Matching

## Problem Statement
`run_silent_inner` uses a hardcoded string `"heartbeat check-in send message reminder"` as synthetic trigger text for skill matching. This is fragile — if skill keywords change, the hardcoded text won't match. It also bypasses the actual context of what the silent agent needs.

## Findings
- Location: `crates/mika-agent/src/agent.rs` — `run_silent_inner` function
- The trigger text is a magic string not derived from actual skill configuration
- Since all skills are `always_on`, this doesn't matter today, but it's a maintenance trap
- If skills ever become keyword-gated, silent mode will silently break

## Proposed Solutions

### Option 1: Use all always_on skills directly (skip matching for silent mode)
- **Pros**: No magic strings, correct by construction
- **Effort**: Small
- **Risk**: Low

### Option 2: Derive trigger text from skill keyword lists
- **Pros**: Always in sync with skill config
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/agent.rs`

## Acceptance Criteria
- [ ] No hardcoded trigger text
- [ ] Silent mode skill selection is correct by construction

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — architecture-strategist agent
