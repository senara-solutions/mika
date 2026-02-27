---
status: pending
priority: p2
issue_id: 305
tags: [code-review, agent-native, architecture]
dependencies: []
---

# Make shell-exec skill always_on for agent-native parity

## Problem Statement

The tmux skill's `system_prompt.md` says "When NOT to use tmux (use shell-exec instead) — Quick one-shot commands." However, shell-exec has `always_on = false` with keywords `["run command", "execute", "shell", "terminal"]`. This creates a capability gap: the agent is told about shell-exec as the preferred alternative for quick commands, but shell-exec may not be available if the user's message doesn't contain trigger keywords.

This is the exact same class of parity gap that tmux just had (PR #25), now applying to shell-exec.

## Findings

- **Agent-native review**: 4 of 6 skills (shell-exec, web-search, file-reader, calendar) suffer the same keyword-gating parity gap
- **Learnings research**: Past solution documented token budget impact of always-on tools (~200 tokens per tool per API call)
- **Tmux prompt references shell-exec** without acknowledging it might be absent

## Proposed Solutions

### Solution A: Make shell-exec always_on = true
- Change `templates/skills/shell-exec/skill.toml` from `always_on = false` to `true`
- Pros: Fixes the parity gap; matches tmux availability
- Cons: Adds ~200 tokens per API call
- Effort: Small
- Risk: Low

### Solution B: Update tmux prompt to not reference shell-exec
- Remove the "use shell-exec instead" guidance from tmux system_prompt.md
- Pros: No token cost increase
- Cons: Doesn't fix the underlying gap; agent loses useful guidance
- Effort: Small
- Risk: Low

## Recommended Action

(To be filled during triage)

## Technical Details

- **Affected files**: `templates/skills/shell-exec/skill.toml`
- **Token impact**: ~200 tokens per API call (similar to tmux)

## Acceptance Criteria

- [ ] shell-exec skill tools available on every agent turn
- [ ] OR tmux prompt updated to not reference shell-exec when it may be absent

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-27 | Created from PR #25 agent-native review | Same parity gap pattern as tmux |

## Resources

- PR #25: https://github.com/senara-solutions/mika/pull/25
- Agent-native review finding (shell-exec parity gap)
