---
title: Always-on skill oversized prompt causes silent degradation
category: integration-issues
date: 2026-03-30
tags: [skills, always_on, prompt, silent-failure, skill-loading]
module: mika-agent/skills
severity: high
---

# Always-on skill oversized prompt causes silent degradation

## Problem

The skill loader silently discarded prompts exceeding `max_prompt_size` (default 16KB), emitting only a WARN log. For `always_on` skills like `self-dev` (29KB prompt), the skill loaded but with an empty prompt — functionally dead. The agent lost its core workflow instructions with no obvious error signal. Discovered via senara-solutions/mika-skills#51 and #53.

## Root Cause

`load_snippet_with_limit()` returned an empty `String` on size limit violation. The calling code in `scan_skills_dir()` created the `SkillEntry` with `prompt_snippet: String::new()`. At runtime, `inject_skills_and_resolve_tools()` saw the empty prompt and skipped injection — the skill's instructions silently disappeared from the system prompt.

This is the same "loaded but broken" anti-pattern seen in tool name shadowing and callback task silent drops.

## Solution

### 1. `SnippetLoadResult` enum replaces stringly-typed return

```rust
enum SnippetLoadResult {
    Ok(String),                      // Successfully loaded
    Empty,                           // File missing or empty (legitimate)
    Oversized { size: u64, limit: u64 }, // File exceeds size limit
    ReadError(String),               // IO error
}
```

Callers can now distinguish "file missing" from "file too large" from "IO error" — critical for policy decisions.

### 2. `scan_skills_dir` enforces always_on invariant

- **`always_on` + Oversized/ReadError**: skill skipped entirely (`error!` log, `skipped_count++`, `continue`)
- **Non-`always_on` + Oversized**: prompt discarded, skill still loads (`error!` log, upgraded from WARN)
- **Empty**: no error — tool-only skills without prompts are valid

### 3. Post-override validation in `apply_overrides()`

After DB overrides are applied, a `retain()` pass checks: if a skill is now `always_on` (via override) AND has an empty prompt AND the on-disk prompt file exceeds the effective limit, the skill is removed from the registry. This catches the edge case where a DB override flips `always_on=true` on a skill whose prompt was already emptied during scan.

### 4. `validate_skill()` differentiated diagnostics

- `always_on`: "skill will be SKIPPED at startup (always_on skills require their prompt to function)"
- Non-`always_on`: "prompt will be EMPTY at startup"

### 5. Model variant prompts — warn and fall back

Oversized model variant prompts warn and fall back to the root prompt (not skip). A variant is an optimization, not a requirement.

### Key files

- `crates/mika-agent/src/skills/index.rs` — `SnippetLoadResult`, `load_snippet_with_limit`, `scan_skills_dir`, `validate_skill`, `scan_provider_variants`
- `crates/mika-agent/src/skills/mod.rs` — `apply_overrides()` post-override validation

## Prevention

- **Always-on skills must not silently degrade.** An `always_on` skill without its prompt is worse than no skill at all — it matches on messages but contributes nothing.
- **Use typed return values instead of sentinel strings.** The previous `load_snippet_with_limit() -> String` collapsed four distinct outcomes into one type. The `SnippetLoadResult` enum makes the failure mode explicit and matchable.
- **Validate after all mutation phases.** The `apply_overrides()` post-check ensures invariants hold even after DB overrides modify `always_on` state. Pre-scan checks alone miss the override edge case.
- **Use `mika skills validate`** to diagnose oversized prompts. The differentiated messaging now tells operators exactly what will happen for their skill type.

## Related

- [Skill prompt snippet size limit configurable](skill-prompt-snippet-size-limit-configurable.md) — the original 8KB→16KB raise that introduced `max_prompt_size`
- [Custom skill silent loading failure](custom-skill-silent-loading-failure.md) — same class of silent-skip problem
- [Skill override persistence via DB layer](../database-issues/skill-override-persistence-via-db-layer.md) — `apply_overrides()` post-scan overlay pattern
- GitHub issue: #331
