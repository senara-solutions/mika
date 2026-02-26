---
status: complete
priority: p2
issue_id: 280
tags: [code-review, prompt, skills]
dependencies: []
---

# Mention create_skill in System Prompt Tool Usage Section

## Problem Statement

The Tool Usage section in the system prompt mentions all other tools but not `create_skill`. The omission means the agent is less likely to proactively suggest skill creation when relevant.

## Findings

- **Agent-native reviewer**: "The Tool Usage section mentions all tools except create_skill"
- Location: `crates/mika-agent/src/prompt.rs` lines 207-221
- Capability Hiding anti-pattern

## Proposed Solutions

### Option A: Add create_skill mention (Recommended)
- Add a line to the Tool Usage section: "You can create new skills using create_skill to extend your capabilities with custom prompt snippets."
- Effort: Trivial (1 line)
- Risk: None

## Acceptance Criteria

- [ ] Tool Usage section mentions create_skill
- [ ] Prompt tests updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Capability Hiding pattern found |
