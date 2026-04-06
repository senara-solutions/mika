---
title: "Per-skill LLM override must filter by MatchReason::Keyword"
category: architecture-patterns
date: 2026-04-07
tags: [skills, llm-override, match-reason, always-on, silent-mode]
issue: 463
related:
  - conditional-required-tools-enforcement-via-match-reason.md
  - per-skill-llm-override-via-toml-section.md
---

# Per-skill LLM override must filter by MatchReason::Keyword

## Problem

When an `always_on` skill declares an `[llm]` section in its `skill.toml`, it silently hijacks the LLM provider for **every agent turn** — regardless of whether the user's message triggered that skill's keywords. Changing the agent's `config.toml` model has no effect because `resolve_skill_llm_override()` considers all matched skills, and `always_on` skills are always in the matched set.

**Symptom:** mika-dev's `self-dev` skill (always_on + `[llm] model = "qwen/qwen3-coder-plus"`) overrode every turn's model, even after `config.toml` was changed to `x-ai/grok-4.1-fast`. The runtime log showed "using per-skill LLM override" on unrelated turns.

## Root Cause

`resolve_skill_llm_override()` accepted `&[&SkillEntry]` — the `MatchReason` was already stripped at call sites. It iterated ALL matched skills including `AlwaysOn` and `Dependency` entries. This was the same class of bug that #265 fixed for `collect_required_tools()`, but the fix was not applied to the LLM override function.

## Solution

Changed `resolve_skill_llm_override()` to accept `&[MatchedSkill<'_>]` and filter to `MatchReason::Keyword` only — exactly mirroring the `collect_required_tools()` pattern:

```rust
fn resolve_skill_llm_override(
    matched: &[MatchedSkill<'_>],  // was &[&SkillEntry]
    settings: Option<&Settings>,
    default_llm: &dyn LlmProvider,
) -> Option<Arc<dyn LlmProvider>> {
    for ms in matched {
        if ms.reason != MatchReason::Keyword {
            continue;  // skip AlwaysOn and Dependency
        }
        // ... rest unchanged
    }
}
```

Three call sites updated:
- **Conversation mode:** Pass `&matched` before extracting `matched_entries`
- **Team mode:** Same pattern
- **Silent mode:** Removed the override call entirely — `safe_always_on_skills()` returns only `AlwaysOn` entries by definition, so no keyword matches can ever occur

## Prevention

When adding new functions that operate on the matched skill set and impose behavioral constraints (overrides, gates, enforcement), always accept `&[MatchedSkill]` and filter by `MatchReason`. The rule: **capabilities** (prompts, tools, timeouts) include all matched skills; **constraints** (required tools, LLM overrides) filter to `Keyword` only.

Functions in this category:
- `collect_required_tools()` — Keyword only (#265)
- `resolve_skill_llm_override()` — Keyword only (#463)
- `inject_skills_and_resolve_tools()` — All matched (provides capabilities)
- `build_skill_tool_map()` — All matched (provides capabilities)
- `max_skill_timeout()` — All matched (provides capabilities)
