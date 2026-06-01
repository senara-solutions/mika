---
title: Always-on skill oversized prompt causes silent degradation
category: integration-issues
date: 2026-03-30
last_updated: 2026-04-18
tags: [skills, always_on, prompt, silent-failure, skill-loading, oversized]
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

### 2. `scan_skills_dir` hard-skips all oversized skills (#630)

All oversized skills are hard-skipped at scan time, regardless of `always_on` status:

- **Oversized (any skill)**: skill skipped entirely (`error!` log, pushed to `ScanResult.skipped`, `continue`). The reason includes size and limit for diagnostics.
- **Empty**: no error — tool-only skills without prompts are valid via `SnippetLoadResult::Empty`
- **ReadError + `always_on`**: skill skipped (prompt is required for always_on)
- **ReadError + non-`always_on`**: prompt emptied, skill still loads (file may appear later)

Previously (before #630), non-`always_on` + Oversized silently emptied the prompt and loaded the skill with its tools but no context — a footgun that produced garbage when triggered.

### 3. Post-override validation removed (#630)

The post-override `retain()` check in `apply_overrides()` that caught "non-always_on oversized + later DB override to always_on" is no longer needed. Since oversized skills are hard-skipped at scan time, they never enter `entries` and cannot be overridden. The edge case is eliminated at the source.

### 4. `validate_skill()` differentiated diagnostics

- `always_on`: "skill will be SKIPPED at startup (always_on skills require their prompt to function)"
- Non-`always_on`: "prompt will be EMPTY at startup"

### 5. Model variant prompts — warn and fall back

Oversized model variant prompts warn and fall back to the root prompt (not skip). A variant is an optimization, not a requirement.

### 6. Three-state startup logging (#630)

`SkillRegistry::log_summary()` emits a single `INFO` line with three counts:

```
INFO skills loaded loaded=16 disabled=9 skipped=1
```

Plus per-skip `WARN` lines with name and reason:

```
WARN skill skipped name=big-skill reason="oversized prompt (29000B, limit 16384B)"
```

Call `log_summary()` after both `apply_overrides()` and `apply_load_safety_check()` for accurate counts that include validation-demoted skills.

### Key files

- `crates/mika-agent/src/skills/index.rs` — `SnippetLoadResult`, `load_snippet_with_limit`, `scan_skills_dir`, `validate_skill`, `scan_provider_variants`
- `crates/mika-agent/src/skills/mod.rs` — `log_summary()`, `apply_overrides()`, `SkillRegistry`
- `crates/mika-agent/src/skills/matcher.rs` — no longer has disabled filter (removed in #630)

## Prevention

- **All oversized skills are hard-skipped.** A skill with tools but no prompt produces garbage — the LLM has tool definitions but no system prompt context to guide their use. Hard-skipping is always safer than loading with an empty prompt.
- **Use typed return values instead of sentinel strings.** The previous `load_snippet_with_limit() -> String` collapsed four distinct outcomes into one type. The `SnippetLoadResult` enum makes the failure mode explicit and matchable.
- **Evict disabled skills at the registry level, not at match time.** `apply_overrides()` removes disabled skills from `entries` before any matching or tool registration runs. No belt-and-suspenders checks needed downstream (#629, #630).
- **Log after all mutation phases.** Call `log_summary()` after both `apply_overrides()` (evicts disabled) and `apply_load_safety_check()` (demotes broken skills to skipped) for accurate three-state counts.
- **Use `mika skills validate`** to diagnose oversized prompts. The differentiated messaging now tells operators exactly what will happen for their skill type.

## Related

- [Skill prompt snippet size limit configurable](skill-prompt-snippet-size-limit-configurable.md) — the original 8KB->16KB raise that introduced `max_prompt_size`
- [Custom skill silent loading failure](custom-skill-silent-loading-failure.md) — same class of silent-skip problem
- [Skill enabled state DB eviction](../architecture-patterns/skill-enabled-state-db-eviction.md) — `apply_overrides()` eviction pattern (#629)
- GitHub issues: #331, #629, #630
