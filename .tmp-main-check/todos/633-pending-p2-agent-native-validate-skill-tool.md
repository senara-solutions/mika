---
status: pending
priority: p2
issue_id: "633"
tags: [code-review, agent-native]
dependencies: []
---

# No Agent-Facing validate_skill Tool

## Problem Statement

The agent can create, update, delete, list, and toggle skills via tools, but has no `validate_skill` tool. When a user asks "check my skills for errors," the agent must punt to a CLI command instead of doing it directly.

## Findings

- Identified by: agent-native-reviewer
- `index::validate_skill()` is already `pub`, takes a `&Path`, and returns `Vec<SkillDiagnostic>` — wrapping it as a builtin tool is ~30 lines

## Proposed Solutions

### Option A: Add validate_skill builtin tool
- Pros: Full agent-native parity for skills management
- Cons: Small additional code
- Effort: Small (~30 lines)
- Risk: None

## Acceptance Criteria

- [ ] Agent can validate skills via a tool call
- [ ] Tool returns structured diagnostics the agent can interpret

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
