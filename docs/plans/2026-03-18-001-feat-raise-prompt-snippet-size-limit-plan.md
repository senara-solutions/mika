---
title: "feat: Raise prompt snippet size limit and make it configurable per-skill"
type: feat
status: completed
date: 2026-03-18
---

# Raise prompt snippet size limit and make it configurable per-skill

## Overview

The `self-dev` skill's `system_prompt.md` (9104 bytes) exceeds the hard-coded 8KB `MAX_PROMPT_SNIPPET_SIZE` limit, causing it to be silently skipped at startup. This breaks the `mika-dev` agent's core development workflow. The fix raises the default to 16KB and adds per-skill `max_prompt_size` override in `skill.toml`.

## Problem Statement

```
WARN prompt snippet exceeds 8KB, skipping path=.../self-dev/system_prompt.md size=9104
```

The 8KB limit was conservative. Legitimate skill prompts (especially `always_on` development skills) can reasonably exceed this. There is no way to override the limit per-skill, so any skill over 8KB is silently broken.

## Proposed Solution

1. **Raise default** from 8KB to 16KB (`MAX_PROMPT_SNIPPET_SIZE = 16 * 1024`)
2. **Per-skill override** via `max_prompt_size` in `skill.toml` (already partially implemented on branch)
3. **Hard ceiling** at 64KB to prevent abuse from marketplace skills
4. **Fix `load_snippet_with_limit`** function signature to accept the limit parameter
5. **Add validation** to `validate_skill()` for prompt snippet size diagnostics
6. **Update warn message** to report actual limit, not hardcoded "8KB"

## Acceptance Criteria

- [x] `MAX_PROMPT_SNIPPET_SIZE` raised from 8KB to 16KB
- [x] `load_snippet_with_limit(path, max_size)` accepts and uses a limit parameter
- [x] `scan_skills_dir()` passes per-skill `max_prompt_size` (clamped to 64KB ceiling) to loader
- [x] Warn message reports actual limit in bytes: `"prompt snippet {size} bytes exceeds limit of {limit} bytes, skipping"`
- [x] `validate_skill()` checks prompt snippet size against effective limit (FAIL if over, WARN if >75%)
- [x] Tests updated: existing `test_snippet_size_limit` passes new signature; new tests for per-skill override, default fallback, ceiling clamp
- [x] `cargo test` passes, `cargo clippy` clean

## Technical Considerations

### Files to modify

- `crates/mika-agent/src/skills/index.rs` — constant, `load_snippet_with_limit`, `scan_skills_dir`, `validate_skill`, tests
- `crates/mika-agent/src/skills/manifest.rs` — already has `max_prompt_size: Option<u64>` (done on branch)

### Edge cases

- `max_prompt_size = 0` → always skip snippet (valid use case)
- `max_prompt_size > 64KB` → clamp to 64KB with warn log
- `max_prompt_size` absent → use 16KB default
- Marketplace skill setting absurd value → clamped by ceiling

### Constants

| Name | Old | New |
|------|-----|-----|
| `MAX_PROMPT_SNIPPET_SIZE` | 8KB (8192) | 16KB (16384) |
| `MAX_PROMPT_SIZE_CEILING` | N/A | 64KB (65536) |

## MVP

### crates/mika-agent/src/skills/index.rs

1. Change `MAX_PROMPT_SNIPPET_SIZE` from `8 * 1024` to `16 * 1024`, add `MAX_PROMPT_SIZE_CEILING = 64 * 1024`
2. Update `load_snippet_with_limit(path: &Path, max_size: u64) -> String` to use `max_size` param
3. Update warn message to include actual numbers
4. In `scan_skills_dir`: clamp `max_size` to `MAX_PROMPT_SIZE_CEILING`
5. In `validate_skill`: add prompt size diagnostic after manifest parse
6. Update all tests

## Sources

- GitHub issue: #199
- `crates/mika-agent/src/skills/index.rs:13` — current constant
- `crates/mika-agent/src/skills/index.rs:477` — `load_snippet_with_limit` function
- `crates/mika-agent/src/skills/manifest.rs:38-40` — `max_prompt_size` field (already on branch)
