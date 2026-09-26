---
status: complete
priority: p3
issue_id: 540
tags: [code-review, agent, prompt]
dependencies: []
---

# SilentTrigger::SkillRun Prompt Framing Too Terse

## Problem Statement

The SkillRun framing says "Execute the skill and process its results" but doesn't tell the agent which tool to call or what parameters to use. Unlike heartbeat/reflection triggers which have clear behavioral instructions, SkillRun leaves the agent guessing.

**Severity:** P3 — Agent may not know how to invoke the named skill.

## Findings

- `crates/mika-agent/src/agent.rs:1202-1208` — terse framing string

## Proposed Solutions

1. **Include skill description and parameters in trigger context**
   - Look up the skill manifest, inject description and default params
   - Effort: Medium
   - Risk: Low

## Acceptance Criteria

- [ ] SkillRun prompt includes skill description and invocation guidance
