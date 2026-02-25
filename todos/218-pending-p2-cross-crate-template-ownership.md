---
status: pending
priority: p2
issue_id: "218"
tags: [code-review, architecture, skills-system]
dependencies: []
---

# Cross-Crate Template Ownership — mika-common Embeds Agent Knowledge

## Problem Statement
`mika-common/src/home.rs` uses `include_str!("../../../templates/skills/...")` to embed skill templates that contain agent-specific tool names and prompt instructions. This makes `mika-common` depend on agent domain knowledge, violating its role as a shared utility crate.

## Findings
- Location: `crates/mika-common/src/home.rs:45-61` — BUILTIN_SKILLS const
- Templates reference tools like `update_core_memory`, `store_fact`, `search_memory` — these are agent concepts
- If tool names change in mika-agent, mika-common templates silently go stale
- The `include_str!` paths reach 3 levels up (`../../../templates/`)

## Proposed Solutions

### Option 1: Move skill seeding to mika-agent crate
- **Pros**: Agent owns its own skill definitions
- **Cons**: Requires mika-agent to expose a bootstrap helper
- **Effort**: Medium
- **Risk**: Low

### Option 2: Move templates into mika-common with explicit documentation
- **Pros**: Simple, just document the coupling
- **Cons**: Doesn't fix the architectural issue
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-common/src/home.rs`, `templates/skills/`

## Acceptance Criteria
- [ ] Skill templates owned by the crate that defines the tools
- [ ] No cross-crate domain knowledge leakage

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — architecture-strategist agent
