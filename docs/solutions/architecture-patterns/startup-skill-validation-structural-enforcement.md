---
title: Startup skill validation — structural enforcement over manual CLI commands
date: 2026-04-13
category: architecture-patterns
module: skills
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding validation checks that only run when explicitly invoked
  - Building skill loading or plugin loading pipelines
  - Designing startup sequences for extensible systems
tags:
  - skills
  - validation
  - startup
  - structural-enforcement
  - skill-registry
---

# Startup skill validation — structural enforcement over manual CLI commands

## Context

`validate_skill()` caught 12+ semantic issues (deprecated `[llm]` sections, missing handler scripts, name-in-keywords, placeholder mismatches), but only ran when an operator explicitly invoked `mika skills validate`. The qa-review skill had an invalid `[llm]` section for 12 hours with zero system notification — the validator existed but nobody ran it.

This is the same anti-pattern documented in `harden-write-skill-variant-no-path-input.md`: relying on prompt-level instructions ("run this command to check") rather than structural enforcement.

## Guidance

Wire validation into the startup path, not just into an explicit CLI command. The implementation pattern:

1. **Add a `validate_loaded()` method** on the registry that calls the existing `validate_skill()` on every loaded entry
2. **Run it after all mutations** (DB overrides, dependency resolution) since mutations can change validation context
3. **Apply a decision matrix** — classify each failure as skip-worthy (skill cannot function) or warn-only (cosmetic/degraded but still operational)
4. **Use a two-phase collect-then-apply pattern** when the registry struct has multiple mutable fields (borrow checker prevents mutating `skipped` inside a `retain()` closure that reads `skills`)
5. **Surface warnings through each mode's natural channel** — TUI gets `ChatRole::System` messages, CLI gets stderr, server gets `tracing::warn`

Key design decisions:

- **`is_skip_worthy_failure()` helper**: Classifies Fail diagnostics by message prefix matching. Skip-worthy: missing handler, broken tools.json, unreadable manifest, oversized always_on prompt. Warn-only: deprecated sections, name-in-keywords, invalid markdown.
- **Catch-all rule**: If `validate_skill()` returns zero Ok diagnostics and at least one Fail, skip the skill regardless of specific failure type. This catches symlink races where the skill directory disappeared between scan and validate.
- **Compute the catch-all BEFORE filtering**: The `all_fail_no_ok` check must use the full diagnostic set, not the filtered issues list. Filtering to Warn+Fail first and then checking "all are Fail" produces false positives — a skill with one Fail and many Ok entries would incorrectly trigger the catch-all.

## Why This Matters

Silent failures in extensible systems compound. The qa-review incident was 12 hours; in production with multiple agents, an invalid skill could degrade service quality indefinitely with no alerting. The validator code already existed — the gap was purely in where it ran.

The principle: if broken state causes real harm, enforce the check at the structural boundary (startup), not at the operator's memory ("remember to run this command").

## When to Apply

- Any time a validation check exists only in an explicit command but not in the loading/startup path
- Plugin/extension systems where invalid configurations should be caught at load time
- Anywhere the validator reads filesystem state that could change between initial load and explicit check

## Examples

Before: validation only on explicit CLI invocation
```
mika skills validate        # catches issues
mika --agent X ask "..."    # silently loads broken skills
```

After: validation at every startup site
```rust
let mut skill_registry = SkillRegistry::from_dir(&skills_dir);
skill_registry.apply_overrides(&overrides);
skill_registry.validate_loaded();  // semantic validation with decision matrix
let skill_registry = Arc::new(skill_registry);
```

## Related

- `harden-write-skill-variant-no-path-input.md` — same principle: structural enforcement over prompt instructions
- `dispatch-readiness-guard-long-running-status-validation.md` — code guards over prompt instructions
- `custom-skill-silent-loading-failure.md` — documents the original silent-skip pattern
- `always-on-skill-oversized-prompt-loud-failure.md` — validate after all mutation phases
- GitHub issue: #530
