---
title: Agent tools must call apply_load_safety_check() on SkillRegistry
date: 2026-04-15
category: best-practices
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Building agent tools that construct a SkillRegistry via from_dir()
  - Reporting skill status or counts to the agent
tags:
  - skills
  - list-skills
  - apply-load-safety-check
  - agent-tools
  - skipped-count
---

# Agent tools must call apply_load_safety_check() on SkillRegistry

## Context

The `list_skills` agent tool constructs a fresh `SkillRegistry` via `from_dir()` and `apply_overrides()` but was not calling `apply_load_safety_check()`. This caused `skipped_count()` to under-report because only parse-time failures (invalid TOML, missing manifest) were counted. Skills with valid manifests but broken runtime structure (missing handler, non-executable handler, broken tools.json) were reported as loaded when the startup paths had actually skipped them.

The three startup sites (`chat.rs`, `ask.rs`, `server/mod.rs`) all follow the pattern `from_dir -> apply_overrides -> apply_load_safety_check`. Any agent tool constructing its own registry must follow the same sequence.

## Guidance

When building a `SkillRegistry` in an agent tool, always call the full initialization sequence:

```rust
let mut registry = SkillRegistry::from_dir(&skills_dir);
registry.apply_overrides(&overrides);
registry.apply_load_safety_check();  // promotes broken-handler skills to skipped
```

The `apply_load_safety_check()` step runs semantic validation (missing handler, broken tools.json, oversized prompts) and moves skip-worthy skills from the loaded list to the skipped list. Without it, `skipped_count()` and `skills()` reflect only parse-time state, not the full validation the agent runtime applies.

## Why This Matters

If an agent tool reports skill status without `apply_load_safety_check()`, the agent sees a different picture than what the runtime actually uses. A skill may appear as "loaded" in the tool output when the startup paths actually skipped it. This prevents agents from self-diagnosing degraded skill registries -- the exact gap the `list_skills` skipped-count feature (#334) was designed to close.

## When to Apply

- Any agent tool or CLI command that constructs `SkillRegistry::from_dir()` and reports skill state
- When adding new skill-listing or skill-status endpoints
- When the reported count or status must match what the running agent actually loaded

## Examples

Before (under-reports skipped count):
```rust
let mut registry = SkillRegistry::from_dir(&skills_dir);
registry.apply_overrides(&overrides);
// Missing apply_load_safety_check() -- skipped_count() only shows parse errors
let count = registry.skipped_count(); // May be 0 when runtime skipped 3
```

After (matches startup behavior):
```rust
let mut registry = SkillRegistry::from_dir(&skills_dir);
registry.apply_overrides(&overrides);
registry.apply_load_safety_check(); // Promotes broken-handler skills to skipped
let count = registry.skipped_count(); // Matches what the running agent skipped
```

## Related

- Issue: #334
- `crates/mika-agent/src/tools/list_skills.rs` -- the tool where this was fixed
- `crates/mika-agent/src/skills/mod.rs` -- `SkillRegistry::apply_load_safety_check()` implementation
- `docs/solutions/architecture-patterns/startup-skill-validation-structural-enforcement.md` -- the validation decision matrix
