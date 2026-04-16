---
title: "fix: Resolve skill dependencies in callback_safe_skills (#578)"
type: fix
status: active
date: 2026-04-16
---

# fix: Resolve skill dependencies in callback_safe_skills (#578)

## Overview

`callback_safe_skills()` returns only `always_on` skills but does not resolve their transitive dependencies. This causes `run_claude_pilot` to be missing from callback turn tool registries because `claude-pilot` (which defines that tool) has `always_on = false` and is only reachable as a dependency of `self-dev`.

## Problem Frame

Issue #567 loosened the silent-mode tool filter so callback turns could use exec/http handler tools. The fix split `safe_always_on_skills()` (which strips exec/http) from `callback_safe_skills()` (which preserves them). Both methods filter on `enabled && always_on` without resolving dependencies.

In conversation mode, `match_skills()` does BFS dependency resolution — `self-dev` (always_on) declares `dependencies = ["claude-pilot"]`, so `claude-pilot` gets pulled in with `MatchReason::Dependency`. In callback mode, `callback_safe_skills()` skips dependency resolution entirely, so `claude-pilot` is never included. The `self-dev` skill's prompt references `run_claude_pilot`, but the tool definition is missing — causing `Unknown tool: run_claude_pilot` at execute time.

Both retry paths (AgentBusy 429 retry and periodic scan retry) flow through `dispatch_resume_agent()` → `run_silent_agent()` → same `callback_safe_skills()` call, so ALL callback turns are affected, not just retries.

## Requirements Trace

- R1. Callback turns must include dependency skills of always-on skills in the tool registry
- R2. Non-callback silent triggers (`Heartbeat`, `Reflection`, `Reminder`, `SkillRun`) must NOT resolve dependencies (security: prevents exec/http handler skills from leaking into autonomous contexts)
- R3. Regression test proves `run_claude_pilot` is available in callback turns when `claude-pilot` is a dependency of an always-on skill

## Scope Boundaries

- Only `callback_safe_skills()` gains dependency resolution; `safe_always_on_skills()` is intentionally left as-is (R2)
- No changes to the agent loop, dispatcher, or retry paths — the tool registry construction is correct; only the skill selection is wrong
- No changes to `match_skills()` in `matcher.rs` — the BFS logic there is reused by reference, not modified

### Deferred to Separate Tasks

- `safe_always_on_skills()` dependency resolution: intentionally excluded; would require filtering resolved deps by handler type, which is a different security model decision

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/skills/mod.rs` lines 347-352: `callback_safe_skills()` — the method to fix
- `crates/mika-agent/src/skills/mod.rs` lines 312-328: `safe_always_on_skills()` — sibling method, NOT changed
- `crates/mika-agent/src/skills/matcher.rs` lines 60-76: BFS dependency resolution in `match_skills()` — reference implementation
- `crates/mika-agent/src/agent.rs` line 2100-2106: trigger-aware match that selects `callback_safe_skills()` vs `safe_always_on_skills()`
- `crates/mika-agent/src/skills/mod.rs` lines 401-438: test helpers `make_entry()` and `make_entry_with_deps()` — already support dependencies

### Institutional Learnings

- `docs/solutions/architecture-patterns/callback-exec-handler-tool-availability.md`: Documents #567's fix. Confirms the exhaustive `match` on `SilentTrigger` prevents regressions; the gap is inside `callback_safe_skills()` itself
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`: Confirms loop prevention is carried by `validate_dispatch_readiness()` and `is_task_context`, NOT by skill filtering — so adding dependency resolution to callback skills is safe
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`: The structural backstop that prevents callbacks from spawning new unrelated long-running tasks remains intact

## Key Technical Decisions

- **Inline BFS in `callback_safe_skills()`** rather than extracting a shared helper: The BFS is 15 lines. Extracting it would couple `callback_safe_skills()` (which returns `Vec<&SkillEntry>`) with `match_skills()` (which returns `Vec<MatchedSkill>` with `MatchReason`). The return types differ enough that a shared helper adds indirection without simplifying either call site. If a third consumer appears, extract then.
- **Resolve dependencies of always-on seeds only**: Start BFS from `enabled && always_on` skills. Resolved dependency skills are included regardless of their `always_on` flag (same as `match_skills()`). Disabled dependency skills break their sub-tree (same as `match_skills()`).

## Open Questions

### Resolved During Planning

- **Should `safe_always_on_skills()` also get dependency resolution?** No. Autonomous triggers must not pull in exec/http handler skills via dependencies. The security model for non-callback silent turns is intentionally restrictive.
- **Does adding dependency skills to callback turns weaken loop prevention?** No. Loop prevention is enforced by `validate_dispatch_readiness()` (4 checks) and `is_task_context: true`, not by skill filtering. See `callback-task-loop-prevention.md`.

### Deferred to Implementation

- Exact variable naming in the BFS loop — follow `match_skills()` conventions

## Implementation Units

- [x] **Unit 1: Add dependency resolution to `callback_safe_skills()`**

**Goal:** Make `callback_safe_skills()` resolve transitive dependencies of always-on skills so that dependency skills (like `claude-pilot`) are included in callback turn tool registries.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/mod.rs`
- Test: `crates/mika-agent/src/skills/mod.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- In `callback_safe_skills()`, after collecting the initial `enabled && always_on` seeds, run BFS over their `dependencies` fields (same algorithm as `matcher.rs` lines 60-76)
- Resolved dependency skills are included if `enabled`, regardless of `always_on` or handler type
- Disabled dependencies break their sub-tree (consistent with `match_skills()`)
- Update the doc comment to reflect that dependencies ARE now resolved (remove the "does NOT resolve" note)
- The method signature stays the same: `pub fn callback_safe_skills(&self) -> Vec<&SkillEntry>`

**Patterns to follow:**
- `match_skills()` BFS at `crates/mika-agent/src/skills/matcher.rs` lines 60-76

**Test scenarios:**
- Happy path: always-on skill A depends on non-always-on skill B; `callback_safe_skills()` returns both A and B
- Happy path: always-on skill A depends on B, B depends on C (transitive); all three returned
- Edge case: dependency skill is disabled; it and its sub-tree are excluded
- Edge case: dependency skill is also `always_on`; no duplicate in result
- Edge case: circular dependency between two skills; no infinite loop (BFS visited-set prevents this)
- Edge case: dependency name doesn't match any loaded skill; silently skipped (consistent with `match_skills()`)
- Regression: `safe_always_on_skills()` still does NOT resolve dependencies (call both methods on same registry, verify different results)
- Regression: `callback_safe_skills()` still includes exec/http handler always-on skills (existing test, verify it still passes)

**Verification:**
- `cargo test -p mika-agent -- skills::tests` passes with new tests
- Existing `test_callback_safe_skills_includes_exec_and_http` and `test_callback_safe_skills_respects_enabled_and_always_on` still pass

## System-Wide Impact

- **Interaction graph:** `callback_safe_skills()` feeds into `inject_skills_and_resolve_tools()` in `run_silent_inner()`. More skills returned means more tools in the LLM's tool definitions and `skill_tool_map`. This is the intended behavior — callback turns should have access to the same tools as conversation mode.
- **Error propagation:** No change. `execute_tool()` dispatch chain is unaffected.
- **State lifecycle risks:** None. Skill selection is read-only; no DB writes.
- **API surface parity:** `safe_always_on_skills()` intentionally remains without dependency resolution (different security model).
- **Unchanged invariants:** The exhaustive `match` on `SilentTrigger` in `run_silent_inner()` is not changed. `validate_dispatch_readiness()` loop prevention guards are not changed. The 4 defense-in-depth guards for callback turns remain intact.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Dependency resolution pulls in unexpected skills with side effects | `validate_dispatch_readiness()` blocks new long-running dispatches; `is_task_context: true` prevents work item creation; these are the actual safety controls, not skill filtering |
| BFS infinite loop on circular dependencies | Use visited set (HashSet of indices), same pattern as `match_skills()` |

## Sources & References

- Related issue: #578
- Related PR: #567 (callback exec handler tools)
- Related code: `crates/mika-agent/src/skills/matcher.rs` (BFS reference)
- Institutional: `docs/solutions/architecture-patterns/callback-exec-handler-tool-availability.md`
- Institutional: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
