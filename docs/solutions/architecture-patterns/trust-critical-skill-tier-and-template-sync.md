---
title: "Trust-critical skill tier and template synchronization for skill-review"
category: architecture-patterns
date: 2026-04-09
tags: [skills, bundled-skills, trust-boundary, review-skill, template-sync]
issue: "#499"
module: bundled_skills, builtin_handlers, skill-review templates
---

# Trust-critical skill tier and template synchronization for skill-review

## Problem

The `skill-review` bundled skill was completely non-functional after the `write_skill_variant` -> `review_skill` handler merge (#477). The Rust handler code was correctly updated, but the compile-time embedded skill templates (`tools.json`, `skill.toml`, `system_prompt.md`) still referenced the deleted `write_skill_variant` tool. This caused:

1. `load_tools_json()` filtered out the tool (not in `KNOWN_BUILTINS`), registering zero tools for the skill
2. `required_tools = ["review_skill", "write_skill_variant"]` made the required-tools gate impossible to satisfy
3. `system_prompt.md` instructed the agent to call a non-existent tool

Additionally, all 12 bundled skills were blocked from review (#486), even functional ones whose prompts could safely be adapted per-model.

## Root Cause

**Template-handler synchronization gap.** The `write_skill_variant` -> `review_skill` merge updated the handler dispatch (`KNOWN_BUILTINS`, `execute()`) but missed the three template files embedded via `include_str!` in `bundled_skills.rs`. These templates define the tool surface sent to the LLM — the handler code being correct was irrelevant if the templates pointed to a non-existent function name.

**Mid-session tool loss (#468) was a misdiagnosis.** Research proved that `match_skills()` runs once before `run_loop()` and the `skill_tool_map` is immutable for the entire loop. Tools cannot disappear mid-session. The "loss" was the tool never being registered in the first place.

## Solution

### 1. Template synchronization

Updated all three files in `crates/mika-agent/templates/skills/skill-review/`:

- **`tools.json`**: Renamed tool from `write_skill_variant` to `review_skill`, added `content` parameter to the schema, updated handler function reference
- **`skill.toml`**: Changed `required_tools` from `["review_skill", "write_skill_variant"]` to `["review_skill"]`
- **`system_prompt.md`**: Full rewrite referencing `review_skill` throughout, added `write_agent_file` prohibition, updated restrictions to trust-critical only

### 2. Two-tier bundled skill classification

Added `TRUST_CRITICAL_SKILLS` constant and `is_trust_critical_skill()` function in `bundled_skills.rs`:

```rust
static TRUST_CRITICAL_SKILLS: &[&str] = &["skill-review", "self-knowledge", "agents-teams"];

pub fn is_trust_critical_skill(name: &str) -> bool {
    TRUST_CRITICAL_SKILLS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}
```

Changed the `review_skill` handler guard and batch mode from `is_bundled_skill()` to `is_trust_critical_skill()`. Other call sites (install, delete, update) remain unchanged — ALL bundled skills are still protected from lifecycle operations.

**Classification criteria:**
- **Trust-critical** (3): Skills whose prompts govern security, identity, or orchestration — model-specific rewording could weaken safety properties
- **Reviewable** (9): Skills whose prompts focus on tool usage mechanics — safe to adapt per-model

### 3. Regression prevention

Added compile-time tests that enforce:
- Every trust-critical skill is a subset of bundled skills
- The system prompt mentions all trust-critical skill names (prevents drift)
- No references to `write_skill_variant` remain in templates
- Reviewable bundled skills are derived from constants (no fragile enumeration)

## Prevention

1. **When merging builtin tools, use the 4-file registration checklist** (from `docs/solutions/architecture-patterns/adding-builtin-handler-skill-git-ops.md`): templates (`tools.json`, `skill.toml`, `system_prompt.md`), handler (`KNOWN_BUILTINS` + `execute()` dispatch), bundled skill registration. Missing any file causes silent failure.

2. **Add prompt-constant sync tests** for any skill that references specific names or lists in its prompt. The `test_skill_review_prompt_lists_trust_critical_skills` pattern catches drift between code constants and embedded prompt text.

3. **Error messages should derive from constants, not duplicate them.** The `trust_critical_skill_names().join(", ")` pattern prevents the error message from drifting when the list changes.

4. **Defense-in-depth gap:** `write_agent_file` has no code-level guard against writing to trust-critical skill directories. Tracked as todo #749.

## Related

- `docs/solutions/architecture-patterns/adding-builtin-handler-skill-git-ops.md` — 4-file registration checklist
- `docs/solutions/security-issues/review-skill-builtin-trust-boundary.md` — original bundled skill guard
- `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md` — variant write path design
- `docs/plans/2026-04-07-004-fix-merge-skill-variant-into-review-plan.md` — the original merge plan
- `todos/749-pending-p2-write-agent-file-trust-critical-bypass.md` — follow-up for `write_agent_file` guard
