---
title: Transient skill enable/disable override via CLI flags
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
  - Suppressing a specific skill for an interactive session
tags: [cli, skill-enable, skill-disable, transient-override, one-shot, skill-activation, apply-transient-always-on, apply-transient-disable]
---

# Transient skill enable/disable override via CLI flags

## Context

The `self-dev` skill is `always_on = true`, meaning every message to mika-dev activates it. This is correct for autonomous paths (webhooks, callbacks, claude-pilot) but noisy for interactive sessions where the user just wants a quick answer. Two CLI flags solve this:

- `--enable-skill <name>` — forces a skill to `always_on = true` for the invocation
- `--disable-skill <name>` — evicts a skill entirely from the registry for the invocation

Both are repeatable, scoped to `mika ask`, and not persisted.

## Guidance

### Pattern: Separate transient methods on SkillRegistry

Two dedicated methods handle transient overrides, separate from `apply_overrides()` which handles DB-backed persistent overrides:

- `apply_transient_disable(&mut self, skill_names: &[String]) -> TransientDisableResult` — evicts named skills from the registry
- `apply_transient_always_on(&mut self, skill_names: &[String]) -> TransientOverrideResult` — sets `always_on = true` on named skills

Call order matters (disable first, enable second — matches `apply_overrides()` Phase 0/1 pattern):
1. `SkillRegistry::from_dir()` — load manifests from disk
2. `apply_overrides()` — apply DB overrides (evicts disabled skills)
3. `apply_transient_disable()` — apply CLI disable overrides (evicts skills)
4. `apply_transient_always_on()` — apply CLI enable overrides (cannot resurrect evicted skills)
5. `apply_load_safety_check()` — validate the final registry state
6. `Arc::new()` — wrap for agent loop

### Conflict detection

Before applying any transient overrides, check for same-skill conflicts between `--enable-skill` and `--disable-skill` (case-insensitive). This is a hard error (`anyhow::bail!`) because the intent is contradictory.

### Transient disable uses full eviction

Setting `always_on = false` would still allow keyword matching. Full eviction from `self.skills` to `self.disabled` prevents all activation paths, consistent with how DB `enabled=false` works in `apply_overrides()` Phase 0.

### Return structured results, not flat Vecs

`TransientOverrideResult` has separate `disabled` and `not_found` fields for accurate user-facing warnings:

- Not found: `"--enable-skill 'X' did not match any loaded skill"`
- Disabled: `"--enable-skill 'X' has no effect -- skill is disabled. Run 'mika skills enable X' first."`

`TransientDisableResult` has a `not_found` field:

- Not found: `"--disable-skill 'X' did not match any loaded skill"`

### Use `conflicts_with` for incompatible modes

Both flags have `conflicts_with = "team"` since team mode builds its own skill registry. This follows the pattern established by `--model`, `--task-id`, and `--task-complete` on `AskArgs`.

## Why This Matters

This pattern separates activation default (manifest) from activation intent (CLI). Skills can default to always-on for autonomous use while interactive users suppress them transiently. The inverse (defaulting off and enabling for autonomous paths) doesn't work because webhook/callback paths have no mechanism to pass CLI flags.

## When to Apply

- Adding a new CLI flag that overrides skill behavior for a single invocation
- Designing any transient override that must stack on top of DB-backed persistent overrides
- Splitting a skill's interactive vs autonomous activation patterns

## Examples

Interactive session with self-dev suppressed:

```bash
mika ask --disable-skill self-dev --agent mika-dev "what's your status?"
```

Force a skill on for a specific invocation:

```bash
mika ask --enable-skill qa-review --agent mika-dev "review PR #45"
```

Both flags combined:

```bash
mika ask --enable-skill qa-review --disable-skill self-dev --agent mika-dev "review this"
```

## Related

- `docs/solutions/architecture-patterns/cli-model-override-one-shot.md` — the `--model` one-shot override pattern that this follows
- `docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md` — scoping flags to specific subcommands
- `docs/solutions/architecture-patterns/skill-enabled-state-db-eviction.md` — precedence: `enabled=false` always wins over `always_on=true`
- GitHub issue: #682 (supersedes #670)
