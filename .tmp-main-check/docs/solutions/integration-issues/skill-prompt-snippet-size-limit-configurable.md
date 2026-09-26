---
title: Skill prompt snippet size limit too low and not configurable
category: integration-issues
date: 2026-03-18
tags: [skills, prompt, system-prompt, configuration, skill-toml]
module: mika-agent/skills
severity: medium
---

# Skill prompt snippet size limit too low and not configurable

## Problem

The hard-coded `MAX_PROMPT_SNIPPET_SIZE` of 8KB silently skipped legitimate skill prompts (e.g., `self-dev` at 9104 bytes). The only symptom was a WARN log at startup — the skill loaded but its prompt was empty, breaking the agent's development workflow.

## Root Cause

`load_snippet_with_limit()` used a single hard-coded constant (8KB) with no per-skill override mechanism. As skills grew more sophisticated, 8KB became too restrictive.

## Solution

1. **Raised default** from 8KB to 16KB (`MAX_PROMPT_SNIPPET_SIZE = 16 * 1024`).
2. **Added per-skill override** via optional `max_prompt_size` field in `skill.toml`:
   ```toml
   [skill]
   name = "large-prompt-skill"
   max_prompt_size = 32768
   ```
3. **Hard ceiling** at 64KB (`MAX_PROMPT_SIZE_CEILING`) — values above are clamped with a warning log. Prevents marketplace skills from loading arbitrarily large prompts.
4. **Updated `load_snippet_with_limit`** to accept a `max_size` parameter instead of using the hard-coded constant.
5. **Added validation** to `validate_skill()` — reports FAIL if snippet exceeds effective limit, WARN if above 75%.

### Key files

- `crates/mika-agent/src/skills/index.rs` — constants, `load_snippet_with_limit`, `scan_skills_dir`, `validate_skill`
- `crates/mika-agent/src/skills/manifest.rs` — `SkillInfo.max_prompt_size: Option<u64>`

## Follow-up: #331 — always_on enforcement

The original fix still allowed oversized prompts to silently degrade `always_on` skills. #331 changed `load_snippet_with_limit()` to return a `SnippetLoadResult` enum (Ok/Empty/Oversized/ReadError) and added enforcement:

- **`always_on` skills:** oversized prompt → skill skipped entirely at startup (`error!` log)
- **Non-`always_on` skills:** oversized prompt → prompt discarded, skill still loads (`error!` log, upgraded from `warn!`)
- **Post-override validation** in `apply_overrides()` catches DB overrides that flip `always_on=true` on skills with already-emptied prompts

## Prevention

- When adding hard-coded limits, provide a per-item override mechanism from the start.
- Use `mika skills validate` to catch oversized prompts before they silently fail at runtime.
- The `validate_skill()` function now reports effective limit vs actual size with differentiated messaging for `always_on` vs non-`always_on` skills.
