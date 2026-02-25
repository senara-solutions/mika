---
status: complete
priority: p3
issue_id: "222"
tags: [code-review, simplicity, skills-system, yagni]
dependencies: ["212", "213"]
---

# ~500 LOC of Speculative Code (Exec/HTTP Handlers, Matcher, tools.json Loader)

## Problem Statement
Approximately 500 lines (53%) of the skills system is speculative infrastructure with zero current users:
- `handler.rs` (235 lines): Exec + HTTP handler dispatch — no exec/http skills exist
- `matcher.rs` (105 lines): Keyword matching — all skills are `always_on`, matcher never filters
- `loader.rs` partial (35 lines): `load_tool_definitions()` — no `tools.json` files exist
- `manifest.rs` partial (60 lines): Exec/Http handler variants + tests
- `agent.rs` (~20 lines): `skill_tool_map` HashMap always empty, threaded through multiple functions

## Findings
- All 3 builtin skills have `always_on = true` → matcher always returns everything
- Zero exec or HTTP skills exist → handler dispatch is dead code
- `skill_tool_map` is always an empty HashMap → lookup always fails
- Tests exist for unreachable code paths (~50+ lines)
- Code-simplicity-reviewer estimates 35-40% of skills code could be removed

## Proposed Solutions

### Option 1: Full YAGNI cleanup
- Delete `handler.rs` entirely (235 lines)
- Delete `matcher.rs`, replace with `skills.iter().collect()` (105 lines)
- Remove `load_tool_definitions()` from loader.rs (35 lines)
- Remove Exec/Http from manifest.rs (60 lines)
- Remove `skill_tool_map` from agent.rs (20 lines)
- **Pros**: 450+ lines removed, simpler, faster, fewer security risks
- **Cons**: Must re-implement when exec/http skills are actually needed
- **Effort**: Medium
- **Risk**: Low (no current users of removed code)

### Option 2: Keep structure, add #[cfg(feature = "external-skills")] gate
- **Pros**: Code exists but isn't compiled by default
- **Cons**: Feature flags add complexity
- **Effort**: Small
- **Risk**: Low

## Recommended Action
Option 1 — Remove speculative code. Also resolves P1 security issues #211, #212, #213 by eliminating the attack surface.

## Technical Details
- **Affected Files**: `handler.rs`, `matcher.rs`, `loader.rs`, `manifest.rs`, `mod.rs`, `agent.rs`

## Acceptance Criteria
- [ ] handler.rs removed or feature-gated
- [ ] Matcher simplified to return all skills
- [ ] skill_tool_map removed from agent loop
- [ ] All tests still pass
- [ ] No exec/http code paths reachable

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — code-simplicity-reviewer agent
