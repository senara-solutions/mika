---
status: pending
priority: p2
issue_id: 304
tags: [code-review, security, architecture]
dependencies: []
---

# Exclude exec-type skill tools from heartbeat/silent mode

## Problem Statement

With tmux now `always_on = true`, the 6 tmux tools are injected into heartbeat/silent agent runs via `always_on_skills()` at `crates/mika-agent/src/agent.rs:677`. Heartbeat runs are autonomous background tasks with no user oversight. Providing unrestricted shell execution tools in autonomous mode is the highest-risk combination identified in the security review.

Additionally, if `send_message` has no sender configured, the agent may repeatedly call it during heartbeats, wasting API calls on guaranteed failures.

## Findings

- **Security**: Prompt injection attacks that plant triggers in the agent's memory or stored facts could cause autonomous command execution during heartbeat with no user present to notice.
- **Token cost**: 6 tmux tool definitions (~600 tokens) are injected into every heartbeat call even though heartbeats would rarely need terminal management.
- **Agent-native review**: Silent prompt tells agent "Use the send_message tool to contact the user" but doesn't condition on whether a sender is configured.

## Proposed Solutions

### Solution A: Filter exec-type tools from silent mode
- In `run_silent_agent`, filter `always_on_skills()` to exclude skills with exec/http handlers
- Pros: Clean separation; heartbeat keeps memory/messaging tools only
- Cons: May need exec tools in future silent scenarios
- Effort: Small
- Risk: Low

### Solution B: Add `heartbeat_safe` flag to skill manifest
- Add `heartbeat_safe = true/false` to `skill.toml`
- Filter on this flag in silent mode
- Pros: Per-skill granularity; explicit opt-in
- Cons: More config surface; needs TOML schema change
- Effort: Medium
- Risk: Low

### Solution C: Guard send_message in silent prompt
- Check if sender is configured before telling agent to use send_message
- Skip heartbeat entirely if no sender AND no telegram configured
- Pros: Prevents wasted API calls
- Cons: Doesn't address tmux-in-heartbeat concern
- Effort: Small
- Risk: Low

## Recommended Action

(To be filled during triage)

## Technical Details

- **Affected files**: `crates/mika-agent/src/agent.rs` (silent agent), `crates/mika-agent/src/prompt.rs` (silent prompt), `crates/mika-agent/src/skills/mod.rs` (always_on_skills)
- **Related**: Security review of PR #25

## Acceptance Criteria

- [ ] Heartbeat/silent mode does NOT include tmux tools in the Claude tool list
- [ ] Silent prompt conditions send_message guidance on sender availability
- [ ] Existing heartbeat tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-27 | Created from PR #25 security review | Heartbeat + unrestricted exec = highest risk |

## Resources

- PR #25: https://github.com/senara-solutions/mika/pull/25
- Security review finding 1A (heartbeat mode includes tmux tools)
- Agent-native review warning (heartbeat send_message loop)
