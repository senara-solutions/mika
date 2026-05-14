---
module: mika-agent/skills
tags: [skills, prompt-size, ceiling, bundled-skills, infrastructure]
problem_type: infrastructure
category: infrastructure
---

# Raise bundled-skill prompt ceiling from 64KB to 80KB

## Problem

The hard ceiling for bundled-skill `system_prompt.md` files (`MAX_PROMPT_SIZE_CEILING`) was 64KB (65,536 bytes). The `self-dev` prompt had grown to 65,248B — only 288B of headroom. Two PRs (#1101, #1103) that each added legitimate content to self-dev were blocked because they pushed past the ceiling.

The ceiling exists as a structural guard: without it, an oversized prompt causes the skill to be silently skipped at startup, leaving the agent in a degraded state. The guard converts silent degradation into a build-time / CI failure.

## Solution

Raised `MAX_PROMPT_SIZE_CEILING` from `64 * 1024` to `80 * 1024` (81,920 bytes). This is a tactical unblock that restores ~22% headroom.

### Files changed

- `crates/mika-agent/src/skills/index.rs` — constant, doc comment, warning message, test comments
- `crates/mika-agent/tests/bundled_skills_load.rs` — remediation hint in failure message
- `docs/skills.md` and `crates/mika-agent/docs/skills.md` — documentation references

### What was NOT changed (and why)

- `MAX_SKILL_TOML_SIZE` stays at 64KB — that's the manifest file size limit, a separate concern
- `INVESTIGATE_BODY_LIMIT` in `server/investigate.rs` — unrelated 64KB constant
- Test data values that construct oversized files — they test the mechanism (100KB vs ceiling), not the specific number

## Follow-up

This is not the permanent target. A structural decomposition of self-dev along the trigger-event axis will bring the prompt back under a tighter ceiling. Once that lands, the ceiling should be re-tightened.

## Verification

```bash
cargo test -p mika-agent --test bundled_skills_load
cargo test -p mika-agent -- test_scan_clamps_override_to_ceiling test_scan_skips_always_on_skill_with_oversized_prompt
```
