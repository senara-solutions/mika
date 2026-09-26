---
title: New bundled skills must be added to mika-relay disabled_skills list
date: 2026-05-08
category: best-practices
module: well-known-agents
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a new skill directory under skills/bundled/
  - Rebasing a branch that introduces a new bundled skill against main
tags:
  - bundled-skills
  - mika-relay
  - well-known-agents
  - disabled-skills
  - relay
---

# New bundled skills must be added to mika-relay disabled_skills list

## Context

When adding a new bundled skill to `skills/bundled/`, the `test_relay_disables_all_bundled_skills_except_permission_policy` test in `well_known_agents.rs` enforces that every bundled skill except `permission-policy` appears in `MIKA_RELAY.disabled_skills`. This test was introduced to prevent new skills from accidentally being active on the mika-relay agent, which is a lightweight agent that only needs `permission-policy`.

This surfaced during the rebase of PR #1005 (dev-handsoff bundled skill, mika#967) against main (mika#1037). The original branch predated the comprehensive test, so the clean rebase passed compilation but failed this test.

## Guidance

When adding a new bundled skill:

1. Add the skill directory under `skills/bundled/<skill-name>/`
2. Add `"<skill-name>"` to `MIKA_RELAY.disabled_skills` in `crates/mika-agent/src/well_known_agents.rs`
3. Place it alphabetically in the engine-coupled section (before the `// Community` comment)

```rust
// In MIKA_RELAY.disabled_skills:
disabled_skills: &[
    // Engine-coupled (skills/bundled/):
    // ...
    "dev-groom",
    "dev-handsoff",  // <-- new skill added here
    // Community (hardcoded BUNDLED_SKILLS):
    // ...
],
```

You do NOT need to add it to `MIKA_DEV.disabled_skills` or `MIKA_QA.disabled_skills` unless the skill belongs to a different agent scope (e.g., architect-only skills are disabled on both dev and qa).

## Why This Matters

The relay agent handles only `can_use_tool` permission relay events from claude-pilot sessions. Any skill beyond `permission-policy` is unnecessary context pollution and a potential trigger source. The comprehensive test ensures this invariant holds as new skills are added — catching the gap at test time rather than in production.

## When to Apply

- Adding any new directory under `skills/bundled/`
- Rebasing a branch that introduces a new bundled skill against a main that has the comprehensive relay test

## Examples

The dev-handsoff skill (mika#967) was a prompt-only, artifact-only skill with no engine dependencies. Despite being fully self-contained, it still needed to be in the relay disabled list because the test exhaustively checks all bundled skill names.

## Related

- mika#1037 — Rebase ticket where this was discovered
- mika#967 — Original dev-handsoff feature ticket
- `docs/solutions/best-practices/operator-only-bundled-skill-structural-enforcement-2026-04-28.md` — Related pattern for operator-only skill enforcement
