---
status: complete
priority: p3
issue_id: "358"
tags: [code-review, documentation, tmux]
dependencies: []
---

# Fix Copy-Mode System Prompt Contradiction

## Problem Statement

The system prompt says copy-mode is "unrecoverable via send-keys" but `send_command.sh` actually attempts auto-recovery via `tmux send-keys -X cancel`. The wording should reflect the auto-recovery attempt.

## Findings

- **Code Simplicity Reviewer**: The system prompt wording contradicts the handler behavior. Either remove auto-recovery or update the prompt.
- **Source**: `templates/skills/tmux/system_prompt.md` line 30, `templates/skills/tmux/handlers/send_command.sh` lines 46-55

## Proposed Solutions

### Solution A: Update system prompt wording (Recommended)
Change "unrecoverable via send-keys" to reflect that auto-recovery is attempted first, with kill/recreate as fallback.

- **Pros**: Accurate, guides agent correctly
- **Cons**: None
- **Effort**: Small
- **Risk**: None

## Recommended Action

Solution A — update wording.

## Acceptance Criteria

- [x] System prompt accurately describes copy-mode auto-recovery behavior

## Work Log

- 2026-02-28: Found during code review, fixed immediately
