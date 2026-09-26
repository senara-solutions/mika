---
status: pending
priority: p3
issue_id: "656"
tags: [code-review, quality]
dependencies: []
---

# `run_gh`: Add `minItems: 1` to JSON Schema for command array

## Problem Statement

The code checks for empty arrays at runtime, but the JSON Schema does not declare `"minItems": 1`. LLMs use the schema to decide what to generate — adding this constraint signals the requirement at schema level, reducing the chance the agent emits an empty array.

## Findings

- **Agent-native reviewer**: Suggested adding `minItems` for better LLM signaling.

## Proposed Solutions

### Solution 1: Add minItems to schema
```json
"command": {
  "type": "array",
  "items": { "type": "string" },
  "minItems": 1,
  "description": "..."
}
```
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/templates/skills/github/tools.json`

## Acceptance Criteria

- [ ] `minItems: 1` added to command array schema

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Flagged by agent-native reviewer |
