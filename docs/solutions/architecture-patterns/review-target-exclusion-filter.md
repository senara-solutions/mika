---
title: Post-match review-target exclusion prevents circular skill activation
date: 2026-04-15
category: architecture-patterns
module: skills/review_filter, agent
problem_type: best_practice
component: assistant
severity: high
applies_when:
  - Adding new post-match filtering steps to the skill pipeline
  - Modifying skill-review activation or skill matching behavior
  - Adding new agent loop paths that call match_message()
tags:
  - skill-matching
  - review-skill
  - circular-activation
  - post-match-filter
  - prompt-injection-prevention
  - match-reason
---

# Post-match review-target exclusion prevents circular skill activation

## Context

When the `skill-review` skill is keyword-triggered (e.g., "review skill self-dev"), the user message may contain keywords that also match the skill being reviewed. If both skills activate, the reviewed skill's prompt is injected into the system prompt alongside the review instructions — the reviewer becomes contaminated by the reviewed skill's behavioral directives.

This is a structural safety concern. Prompt-level guards were already in place (3-call cap per skill in the skill-review system prompt), but institutional learning from the deterministic-context-injection work proved that prompt enforcement fails when the LLM controls the critical path. Code-level structural enforcement was required.

## Guidance

The solution uses **post-match filtering** — the established pattern in the agent loop for removing skills from the matched set before prompt injection. The filter lives in `skills/review_filter.rs` and exposes a single public function: `apply_review_filter(&mut matched, user_message)`.

**Pipeline position:** The filter runs after `match_message()` and before `resolve_contexts()`. This is the earliest safe point — excluded skills do not participate in context resolution, LLM override selection, required_tools collection, or prompt injection.

**Activation conditions:**
1. `skill-review` must be keyword-matched (not AlwaysOn or Dependency) in the current turn
2. Only other keyword-matched skills are candidates for exclusion (AlwaysOn and Dependency skills are never excluded)
3. The candidate skill's name must appear in the user message (case-insensitive substring match)

**Applied in both paths:** The filter is wired into conversation mode (`run_agent_inner`) and team mode (`run_team_agent_inner_impl`). Silent mode is correctly exempt — it uses `safe_always_on_skills()` / `callback_safe_skills()` which never keyword-match, making the filter a no-op.

## Why This Matters

Without this filter, reviewing a skill that has keywords matching the user message (e.g., "review skill self-dev" where `self-dev` has keyword "self-dev") causes both `skill-review` and `self-dev` to activate. The `self-dev` prompt is then injected into the system prompt alongside review instructions, which can:

1. Contaminate the review with the reviewed skill's behavioral directives
2. Cause the agent to execute the reviewed skill's workflow instead of objectively reviewing its prompt
3. Create circular activation where the reviewer is influenced by the very content it should be evaluating

## When to Apply

- **New agent loop paths:** Any new code path that calls `match_message()` must also call `apply_review_filter()` before the matched set is used for context resolution or prompt injection
- **New post-match filters:** Follow the same pattern — accept `&[MatchedSkill]`, return indices in descending order, remove at the call site or provide a convenience `apply_*` wrapper
- **Skill matching changes:** If the matching algorithm changes (e.g., new MatchReason variants), verify the review filter still correctly identifies keyword-matched skills

## Examples

The filter follows the existing `context_exclude` pattern in `agent.rs`:

```rust
// Before (context exclusion — existing pattern):
let (resolved_context, context_exclude) = context::resolve_contexts(...).await;
for &idx in context_exclude.iter().rev() {
    matched.remove(idx);
}

// After (review-target exclusion — same pattern, new step):
review_filter::apply_review_filter(&mut matched, params.user_message);
// Then context resolution, LLM override, prompt injection...
```

The `apply_review_filter` function encapsulates the detection, index collection, logging, and removal in one call — no need for callers to manage descending-index removal.

## Related

- Issue #513 (this feature), parent #512
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — pipeline ordering
- `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md` — MatchReason filtering precedent
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — same class of structural-over-prompt defense
- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` — complementary prompt-level cap
- `docs/solutions/security-issues/review-skill-builtin-trust-boundary.md` — trust-critical skill guards
