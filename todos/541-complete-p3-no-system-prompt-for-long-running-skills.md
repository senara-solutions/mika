---
status: complete
priority: p3
issue_id: 541
tags: [code-review, agent, prompt]
dependencies: []
---

# No System Prompt Guidance About Long-Running Skills

## Problem Statement

When an agent invokes a long-running skill, it receives "Task submitted (long-running). ID: {task_id}" but has no prior context about what this means, whether to inform the user, or how to check status later. The system prompt has no mention of long-running skills.

**Severity:** P3 — Context starvation for agent.

## Findings

- `crates/mika-agent/src/prompt.rs` — Tool Usage section (~line 275) has no mention of long-running skills
- `crates/mika-agent/src/skills/executor.rs` — returns task submission message

## Proposed Solutions

1. **Add guidance to system prompt**
   - "Some tools are long-running and return a task ID instead of immediate results. When this happens, inform the user that a background task is running and you'll follow up when results arrive. Do not retry the tool."
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] System prompt includes long-running skill guidance
- [ ] Agent informed to tell user about background task, not retry the tool
