---
status: pending
priority: p2
issue_id: "216"
tags: [code-review, architecture, skills-system]
dependencies: []
---

# Behavior Divergence When No Skills Directory Exists

## Problem Statement
The agent has a 3-way branch: (1) matched skills → use skill tools + snippets, (2) no skills dir → all builtin tools with NO instructions, (3) skills exist but none match → no tools at all. Case 2 gives tools without instructions (degraded). Case 3 gives no tools (broken). This is confusing and error-prone.

## Findings
- Location: `crates/mika-agent/src/agent.rs` — the `if !matched.is_empty() / else if !params.skills.has_skills() / else` branch
- Fallback (no skills dir) provides tools but no prompt instructions → agent loses guidance
- No-match branch provides NO tools → agent is helpless
- Since all builtin skills are `always_on`, case 3 never triggers today, but it's a latent bug

## Proposed Solutions

### Option 1: Always include builtin tools regardless of skill matching
- **Pros**: Robust, never loses tools
- **Cons**: Overrides skill-based filtering intent
- **Effort**: Small
- **Risk**: Low

### Option 2: Keep base instructions in prompt.rs, only add skill-specific extras
- **Pros**: Instructions always present, skills augment rather than replace
- **Cons**: Slightly more prompt tokens
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/agent.rs`, `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria
- [ ] Agent always has builtin tools available
- [ ] Base instructions always present in system prompt
- [ ] Skill snippets augment, not replace

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — architecture-strategist agent
