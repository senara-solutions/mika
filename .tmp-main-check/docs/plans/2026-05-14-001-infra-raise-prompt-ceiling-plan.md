# Plan: Raise bundled-skill prompt ceiling from 64KB to 80KB

**Ticket:** mika issue#1105
**Type:** infra
**Date:** 2026-05-14

## Context

The hard ceiling for bundled-skill `system_prompt.md` files is 64KB (65,536 bytes), enforced by `MAX_PROMPT_SIZE_CEILING` in `crates/mika-agent/src/skills/index.rs`. The `self-dev` prompt is at 65,248B — 288B of headroom. Two open PRs (#1101, #1103) each push past the limit and are blocked.

This is a tactical unblock: raise the ceiling to 80KB (81,920 bytes) to restore ~22% headroom while a structural decomposition of self-dev is planned separately.

## Scope

**In scope:**
1. Raise `MAX_PROMPT_SIZE_CEILING` constant from `64 * 1024` to `80 * 1024`
2. Update all user-facing strings that reference "64KB" in the context of the prompt ceiling
3. Update documentation that references the 64KB ceiling

**Out of scope:**
- `MAX_SKILL_TOML_SIZE` stays at `64 * 1024` (separate concern — manifest file size, not prompt size)
- `INVESTIGATE_BODY_LIMIT` in `server/investigate.rs` (unrelated 64KB constant)
- Test data values that construct oversized files for testing (these test the mechanism, not the specific number)

## Changes

### 1. `crates/mika-agent/src/skills/index.rs` — Constants and messages

| Location | Current | New |
|----------|---------|-----|
| Line 22: `MAX_PROMPT_SIZE_CEILING` constant | `64 * 1024` | `80 * 1024` |
| Line 20: Doc comment | "Hard ceiling … (64 KB)" | "Hard ceiling … (80 KB)" |
| Line 438: warn message | `"skill.toml exceeds 64KB, skipping"` | No change — this references `MAX_SKILL_TOML_SIZE`, not the prompt ceiling |
| Line 442: reason format | `"skill.toml exceeds 64KB ({}B)"` | No change — same, `MAX_SKILL_TOML_SIZE` |
| Line 519: error message | `"ceiling: 64KB"` | `"ceiling: 80KB"` |
| Line 662: diagnostic | `"skill.toml exceeds 64KB"` | No change — `MAX_SKILL_TOML_SIZE` |

### 2. `crates/mika-agent/tests/bundled_skills_load.rs` — Regression test

| Location | Current | New |
|----------|---------|-----|
| Line 57-58: remediation hint | `"ceiling 64KB"` | `"ceiling 80KB"` |

### 3. `crates/mika-agent/src/skills/index.rs` — Test comments

| Location | Current | New |
|----------|---------|-----|
| Line 2172: test comment | `"100KB prompt — over 64KB ceiling"` | `"100KB prompt — over 80KB ceiling"` |
| Line 2273: test comment | `"29KB prompt — over 16KB default but under 64KB ceiling"` | `"29KB prompt — over 16KB default but under 80KB ceiling"` |

### 4. Documentation updates

| File | Current | New |
|------|---------|-----|
| `crates/mika-agent/docs/skills.md` line 60 | `"Clamped to a 64KB ceiling"` | `"Clamped to an 80KB ceiling"` |
| `crates/mika-agent/docs/skills.md` line 380 | `"hard ceiling of **64KB**"` | `"hard ceiling of **80KB**"` |
| `crates/mika-agent/docs/architecture.md` line 277 | `"max 64KB"` — this refers to skill.toml, not prompt | No change |

### 5. No test logic changes needed

The existing tests construct oversized files relative to `MAX_PROMPT_SIZE_CEILING` implicitly:
- `test_scan_skips_oversized_manifest` — tests `MAX_SKILL_TOML_SIZE` (65KB file vs 64KB limit), unchanged
- `test_scan_skips_prompt_over_ceiling` — tests 100KB prompt, still over 80KB ceiling, test still passes
- `test_scan_loads_custom_max_prompt_size` — tests 29KB prompt under ceiling with `max_prompt_size = 65536`, still under 80KB, test still passes

The test at line 2269 uses `max_prompt_size = 65536` which is a per-skill override (clamped to ceiling). With ceiling at 80KB, 65536 is still under ceiling so the clamp is a no-op and the behavior is identical. No change needed.

## Verification

```bash
cargo test -p mika-agent --test bundled_skills_load
cargo test -p mika-agent -- test_scan_skips_oversized_manifest test_scan_skips_prompt_over_ceiling test_scan_loads_custom_max_prompt_size
```

## Risk

Low. Pure constant change with documentation alignment. No behavioral change for any prompt currently under 64KB. The only behavioral change is that prompts between 64KB and 80KB that were previously rejected will now load.
