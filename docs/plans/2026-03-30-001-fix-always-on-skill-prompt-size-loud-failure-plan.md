---
title: "fix: Fail loudly when always_on skill prompt exceeds size limit"
type: fix
status: active
date: 2026-03-30
---

# fix: Fail loudly when always_on skill prompt exceeds size limit

## Overview

The skill loader silently discards prompts exceeding `max_prompt_size` (default 16KB), emitting only a WARN log. For `always_on` skills, this silently breaks the entire agent workflow — the skill loads but with an empty prompt, making it functionally dead. Discovered via `self-dev` skill (29KB) being silently dropped.

## Problem Statement

`load_snippet_with_limit()` returns an empty string when the prompt file exceeds the size limit. The calling code in `scan_skills_dir()` creates the `SkillEntry` anyway with `prompt_snippet: String::new()`. At runtime, `inject_skills_and_resolve_tools()` sees the empty prompt and skips injection — the `always_on` skill's instructions silently disappear from the system prompt.

This is a recurring "loaded but broken" anti-pattern (see also: tool name shadowing, callback tasks silently dropped).

## Proposed Solution

### 1. Change `load_snippet_with_limit` return type

**File:** `crates/mika-agent/src/skills/index.rs` (~L1286-1299)

Replace `fn load_snippet_with_limit(path, max_size) -> String` with a result type that distinguishes failure modes:

```rust
enum SnippetLoadResult {
    Ok(String),           // Successfully loaded
    Empty,                // File missing or empty (legitimate)
    Oversized { size: u64, limit: u64 }, // File exceeds size limit
    ReadError(String),    // IO error reading file
}

fn load_snippet_with_limit(path: &Path, max_size: u64) -> SnippetLoadResult
```

This lets callers distinguish "prompt was oversized" from "no prompt file" — critical for deciding whether to skip an `always_on` skill.

### 2. Fail loudly in `scan_skills_dir` for always_on skills

**File:** `crates/mika-agent/src/skills/index.rs` (~L268-283)

After calling `load_snippet_with_limit`, match on the result:

- **`Oversized` + `manifest.skill.always_on == true`**: Log `error!` with skill name, size, limit. Increment `skipped_count`. `continue` (skip skill entirely).
- **`Oversized` + `manifest.skill.always_on == false`**: Log `error!` (upgraded from WARN). Set `prompt_snippet = String::new()`. Continue loading the skill (existing behavior, just louder).
- **`Empty`**: Set `prompt_snippet = String::new()`. No error — tool-only skills without prompts are valid.
- **`Ok(content)`**: Use the content as normal.
- **`ReadError`**: Log `error!`, increment `skipped_count`, `continue`.

### 3. Post-override validation inside `apply_overrides()`

**File:** `crates/mika-agent/src/skills/mod.rs` — `apply_overrides()`

After applying all DB overrides, iterate the skills vec. For any entry where `always_on == true && prompt_snippet.is_empty()`:

- Check if the skill has a `system_prompt.md` file on disk that exceeds the effective size limit (to distinguish "no prompt file" from "oversized prompt").
- If oversized: log `error!` naming the skill, remove it from the registry (or mark disabled). This catches the edge case where a DB override flips `always_on = true` on a skill whose prompt was already silently emptied.
- If no prompt file: allow — tool-only `always_on` skills are valid.

This validation runs automatically in all paths: startup, hot-reload, delegate task, team engine.

### 4. Update `validate_skill()` diagnostics

**File:** `crates/mika-agent/src/skills/index.rs` — `validate_skill()` (~L659-695)

Differentiate the oversized-prompt diagnostic message based on `always_on`:

- `always_on = true`: "FAIL: prompt exceeds size limit — skill will be SKIPPED at startup (always_on skills require their prompt to function)"
- `always_on = false`: "FAIL: prompt exceeds size limit — prompt will be EMPTY at startup"

### 5. Model variant prompts — fallback, don't fail

**File:** `crates/mika-agent/src/skills/index.rs` — `scan_provider_variants()` (~L1147)

When a model variant prompt exceeds the limit on an `always_on` skill, log WARN and fall back to root prompt (existing behavior). Do NOT skip the skill — the root prompt is the functional prompt; variants are enhancements.

## Acceptance Criteria

- [x] `always_on` skill with oversized root prompt is skipped entirely at startup (not loaded with empty prompt)
- [x] Non-`always_on` skill with oversized prompt still loads (empty prompt) but logs at ERROR level
- [x] DB override flipping `always_on = true` on a skill with oversized (empty) prompt triggers post-override validation and removes the skill
- [x] `mika skills validate` differentiates messaging for `always_on` vs non-`always_on` skills
- [x] Model variant oversized prompt falls back to root without skipping the skill
- [x] Existing tests pass; new tests cover all new branches
- [x] `SkillRegistry::from_dir()` startup warning includes names of skipped `always_on` skills

## Technical Considerations

- **`apply_overrides()` is the enforcement point for DB overrides.** Embedding validation here guarantees coverage across all 4+ call sites (startup, server hot-reload, a2a hot-reload, delegate_task).
- **`safe_always_on_skills()` filtering is NOT a failure.** Excluding exec/http handlers in silent mode is a security boundary, not a loading error. The proposed changes do not affect this path.
- **Bundled skills are compile-time embedded.** Their sizes are fixed and controlled by developers. No runtime check needed, but a CI lint could catch future issues.
- **Disabled skills (`.disabled` marker):** The disabled check currently runs after prompt loading. A disabled skill with an oversized prompt would be skipped before the disabled check runs. This is acceptable — the error surfaces the real problem when re-enabled.

## Files to Modify

- `crates/mika-agent/src/skills/index.rs` — `load_snippet_with_limit()`, `scan_skills_dir()`, `validate_skill()`, `scan_provider_variants()`
- `crates/mika-agent/src/skills/mod.rs` — `apply_overrides()`, `from_dir()`

## Sources

- GitHub issue: #331
- Prior solution: `docs/solutions/integration-issues/skill-prompt-snippet-size-limit-configurable.md` — documents the 8KB→16KB raise and per-skill `max_prompt_size` override
- Prior solution: `docs/solutions/integration-issues/custom-skill-silent-loading-failure.md` — documents the same silent-skip class of problem
- Prior solution: `docs/solutions/database-issues/skill-override-persistence-via-db-layer.md` — documents `apply_overrides()` post-scan overlay pattern
