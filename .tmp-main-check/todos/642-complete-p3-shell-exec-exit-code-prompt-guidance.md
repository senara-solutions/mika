---
status: complete
priority: p3
issue_id: 642
tags: [code-review, agent-native, skills]
dependencies: []
---

# Add prompt guidance about shell exit code semantics

## Problem Statement

After #100, shell commands return `Exit code: N` prefix on non-zero exits instead of `is_error=true`. The agent has no prompt-level guidance about this convention, so it must discover the pattern from tool results.

## Findings

- Shell-exec skill description (`skill.toml`) does not mention exit code handling
- System prompt does not mention how to interpret `Exit code:` prefix
- Agent-native review recommends adding guidance so the agent reliably interprets non-zero exits

## Proposed Solutions

### Option A: Add to shell-exec skill description
Add a note to `templates/skills/shell-exec/skill.toml` description field.
- Pros: Co-located with the tool, always visible when skill is active
- Cons: Only covers shell-exec, not other exec handlers
- Effort: Small

### Option B: Add to system prompt skill-exec section
Add guidance in the prompt assembly about interpreting exec handler exit codes.
- Pros: Covers all exec handlers
- Cons: Uses system prompt budget
- Effort: Small

## Acceptance Criteria

- [ ] Agent has prompt-level awareness of exit code prefix format
- [ ] Agent knows non-zero exit is not necessarily an error
