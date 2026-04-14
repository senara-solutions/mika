---
status: pending
priority: p2
issue_id: 762
tags: [code-review, docs, agent-core]
dependencies: []
---

# `callback-task-loop-prevention.md` solution doc claims structural backstop that no longer exists

## Problem Statement

`docs/solutions/architecture-patterns/callback-task-loop-prevention.md` asserts that "callback agents must never have exec/http skills" and points to `safe_always_on_skills()` as a structural backstop. PR #567 relaxes that backstop for `SilentTrigger::Callback` turns (exec/http handlers are now included via `callback_safe_skills()`). The doc is now stale and misleading — future readers may rely on a property that no longer holds.

## Findings

- **architecture-strategist review** flagged this as the one real follow-up worth doing in this PR.
- **Evidence:** doc references lines 69, 165, 192 — assertions about exec/http skills being structurally excluded from callbacks.

## Proposed Solution

Update `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` to:

1. Replace "callback agents must never have exec/http skills" claims with "callback agents inherit the same exec/http skill surface the agent already had in conversation mode; dispatch-readiness guard (#525) and work-item state machine prevent callback-spawned long-running task loops."
2. Point the structural-backstop argument at `validate_dispatch_readiness()` in `skills/executor.rs` (the `work_item_active_dispatch` and `work_item_not_dispatchable` rejections) instead of `safe_always_on_skills()`.
3. Cross-reference the new `callback_safe_skills()` doc-comment and #567.

**Pros:** Keeps institutional knowledge accurate.
**Cons:** None.
**Effort:** Small (doc edit, ~30 lines changed).
**Risk:** None.

## Acceptance Criteria

- [ ] The "callback agents must never have exec/http skills" claim is removed or qualified
- [ ] The structural-backstop argument now references `validate_dispatch_readiness()` / #525
- [ ] Cross-reference to `callback_safe_skills()` and #567 added

## Resources

- PR branch: `fix/567/callback-exec-handler-tools`
- Issue: senara-solutions/mika#567
- File: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
