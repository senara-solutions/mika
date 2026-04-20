---
title: Transient skill always_on override via CLI flag
date: 2026-04-20
category: architecture-patterns
module: mika-cli, skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding a CLI flag that overrides skill activation for a single invocation
  - Decoupling a skill's default activation from its autonomous dispatch behavior
  - Designing transient overrides that stack on top of persistent DB overrides
tags: [cli, skill-always-on, transient-override, one-shot, skill-activation, apply-transient-always-on]
---

# Transient skill always_on override via CLI flag

## Context

The `self-dev` skill was `always_on = true`, meaning every message to mika-dev activated it — even simple questions. Interactive sessions were frustrating because a large orchestration prompt was injected unconditionally. The autonomous dev loop (`claude-pilot`) always knows it wants `self-dev` and can explicitly request it.

The solution: flip `self-dev` to `always_on = false` by default and add `--skill-always-on <name>` to `mika ask` so autonomous callers can force specific skills on per-invocation.

## Guidance

### Pattern: Transient override method on SkillRegistry

Add a dedicated `apply_transient_always_on(&mut self, skill_names: &[String]) -> TransientOverrideResult` method, separate from `apply_overrides()` which handles DB-backed persistent overrides. This keeps transient (CLI) and persistent (DB) concerns cleanly separated.

Call order matters:
1. `SkillRegistry::from_dir()` — load manifests from disk
2. `apply_overrides()` — apply DB overrides (evicts disabled skills)
3. `apply_transient_always_on()` — apply CLI overrides (cannot resurrect evicted skills)
4. `validate_loaded()` — validate the final registry state
5. `Arc::new()` — wrap for agent loop

### Return a structured result, not a flat Vec

The initial implementation returned `Vec<String>` for unresolved names. Code review revealed this was insufficient — the caller couldn't distinguish "skill not found" from "skill disabled." The fix introduced `TransientOverrideResult` with separate `disabled` and `not_found` fields, enabling accurate user-facing warnings:

- Not found: `"--skill-always-on 'X' did not match any loaded skill"`
- Disabled: `"--skill-always-on 'X' has no effect — skill is disabled. Run 'mika skills enable X' first."`

### Use `conflicts_with` for incompatible modes

The `--skill-always-on` flag has no effect in `--team` mode (team builds its own skill registry). Rather than silently dropping the flag, add `conflicts_with = "team"` to the clap attribute so users get a clear error at parse time. This follows the pattern established by `--model`, `--task-id`, and `--task-complete` on `AskArgs`.

## Why This Matters

This pattern separates activation default (manifest) from activation intent (CLI). Skills can default to keyword-triggered for interactive use while autonomous workflows explicitly lock in the skills they need. Without this separation, either interactive users suffer noise (always-on) or autonomous callers lose determinism (keyword-only).

## When to Apply

- Adding a new CLI flag that overrides skill behavior for a single invocation
- Designing any transient override that must stack on top of DB-backed persistent overrides
- Splitting a skill's interactive vs autonomous activation patterns

## Examples

Autonomous dispatch with explicit skill activation:

```bash
mika ask --skill-always-on self-dev --agent mika-dev "implement mika issue#123"
```

Multiple skills forced on:

```bash
mika ask --skill-always-on self-dev --skill-always-on qa-review --agent mika-dev "review PR #45"
```

## Related

- `docs/solutions/architecture-patterns/cli-model-override-one-shot.md` — the `--model` one-shot override pattern that this follows
- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — scoping flags to specific subcommands
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — precedence: `enabled=false` always wins over `always_on=true`
- GitHub issue: #670
