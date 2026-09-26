---
status: complete
priority: p2
issue_id: "042"
tags: [code-review, bug, agent, rust-v2]
dependencies: []
---

# is_onboarding Flag Never Refreshed in CLI Loop

## Problem Statement
The `is_onboarding` flag is computed once at CLI startup and never rechecked. After the agent updates `user_summary` during the first conversation, subsequent messages in the same session still see `is_onboarding=true`, which: (1) exempts the agent from core memory rate limits, and (2) injects the onboarding prompt when it's no longer needed.

**Location:** `crates/mika-agent/src/cli.rs:80-83` (computed once, used in every iteration of the loop at line 124)

**Reported by:** security-sentinel, pattern-recognition-specialist, code-simplicity-reviewer

## Findings
- `is_onboarding` checks if `user_summary == "New user. No information yet."` at line 80-83
- The value is captured before the loop starts and never updated
- During onboarding, the agent edits `user_summary` via `update_core_memory`, but `is_onboarding` remains true
- Rate limit exemption means the agent can make unlimited core memory edits for the rest of the session
- Additionally, `core_memory_edit_count` AtomicU32 is created fresh per `run_agent` call (agent.rs:93), so it resets per message anyway

## Proposed Solutions

### Option A: Re-evaluate per iteration (Recommended)
Move the `is_onboarding` check inside the loop, before each `run_agent` call.
- **Pros:** Always accurate, simple change
- **Cons:** Extra DB query per message (trivial cost)
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] `is_onboarding` is re-evaluated before each agent turn
- [ ] After agent updates user_summary, next turn sees `is_onboarding=false`
- [ ] Rate limiting applies correctly after onboarding completes

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Also noted: edit counter resets per message, not per session |
