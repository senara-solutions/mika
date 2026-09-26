---
title: "fix: resolve_skill_llm_override should filter by MatchReason::Keyword only"
type: fix
status: completed
date: 2026-04-07
---

# fix: resolve_skill_llm_override should filter by MatchReason::Keyword only

## Overview

`resolve_skill_llm_override()` in `agent.rs` iterates ALL matched skills (including `always_on`) when collecting `[llm]` overrides. An `always_on` skill with a hardcoded `[llm]` section hijacks the LLM provider for **every turn**, silently overriding agent `config.toml` changes. This is the same class of bug that #265 fixed for `collect_required_tools()`.

## Problem Statement

When `self-dev` skill (`always_on = true`) declares `[llm] provider = "openrouter", model = "qwen/qwen3-coder-plus"`, every mika-dev turn uses that model — even when the agent config was changed to `x-ai/grok-4.1-fast`. The user has no visibility into why their model change didn't take effect.

**Root cause:** `resolve_skill_llm_override()` takes `&[&SkillEntry]` (match reason already stripped at call sites), while `collect_required_tools()` correctly takes `&[MatchedSkill]` and filters to `Keyword` only.

## Proposed Solution

Mirror the `collect_required_tools()` pattern: change `resolve_skill_llm_override()` to accept `&[MatchedSkill]` and filter to `MatchReason::Keyword` only.

## Acceptance Criteria

- [x] `resolve_skill_llm_override()` only considers keyword-matched skills' `[llm]` overrides
- [x] `always_on` skills' `[llm]` sections do NOT apply unless the user's message also triggers their keywords
- [x] Silent mode (heartbeat/reflection) correctly skips LLM override (no keyword matches in background mode)
- [x] Team mode correctly filters by keyword
- [x] Existing conflict detection and same-provider short-circuit logic unchanged
- [x] Unit tests cover: keyword-only override, always_on ignored, mixed set filtering
- [x] `cargo test` and `cargo clippy` pass

## MVP

### 1. Change function signature — `crates/mika-agent/src/agent.rs`

```rust
// BEFORE (line ~2435)
fn resolve_skill_llm_override(
    matched: &[&SkillEntry],
    settings: Option<&Settings>,
    default_llm: &dyn LlmProvider,
) -> Option<Arc<dyn LlmProvider>>

// AFTER
fn resolve_skill_llm_override(
    matched: &[MatchedSkill<'_>],
    settings: Option<&Settings>,
    default_llm: &dyn LlmProvider,
) -> Option<Arc<dyn LlmProvider>>
```

Add keyword filter at the top of the iteration:

```rust
for ms in matched {
    if ms.reason != MatchReason::Keyword {
        continue;
    }
    let entry = ms.entry;
    if entry.manifest.llm.is_empty() {
        continue;
    }
    // ... rest unchanged
}
```

### 2. Update conversation mode call site (line ~1147)

```rust
// BEFORE
let matched_entries: Vec<&SkillEntry> = matched.iter().map(|m| m.entry).collect();
let skill_llm_override = resolve_skill_llm_override(&matched_entries, params.settings, llm);

// AFTER
let skill_llm_override = resolve_skill_llm_override(&matched, params.settings, llm);
let matched_entries: Vec<&SkillEntry> = matched.iter().map(|m| m.entry).collect();
```

Move `matched_entries` extraction AFTER the override call (it's still needed for `inject_skills_and_resolve_tools` etc.).

### 3. Update team mode call site (line ~2268)

Same pattern as conversation mode — pass `&matched` before extracting `matched_entries`.

### 4. Silent mode call site (line ~1945)

`safe_always_on_skills()` returns `Vec<&SkillEntry>` (no `MatchedSkill` wrapper). All entries are `AlwaysOn` by definition. After the fix, passing these would produce zero overrides (correct behavior — background mode should use the agent's default model).

**Option:** Remove the `resolve_skill_llm_override` call entirely in silent mode, since `safe_always_on_skills()` can never produce keyword matches. Replace with a comment explaining why.

### 5. Add tests — `crates/mika-agent/src/agent.rs` (inline test module)

```rust
#[cfg(test)]
mod tests {
    // Test: keyword-matched skill with [llm] → override applies
    // Test: always_on skill with [llm] → override does NOT apply
    // Test: always_on + keyword hit on same skill → override applies (reason is Keyword)
    // Test: empty matched set → None
    // Test: multiple keyword skills with same [llm] → dedup works
    // Test: multiple keyword skills with different [llm] → conflict fallback
}
```

## Sources

- **Pattern precedent:** `collect_required_tools()` at `agent.rs:2554` (Keyword-only filter per #265)
- **Solution doc:** `docs/solutions/architecture-patterns/conditional-required-tools-enforcement-via-match-reason.md`
- **Solution doc:** `docs/solutions/architecture-patterns/per-skill-llm-override-via-toml-section.md`
- **Issue:** senara-solutions/mika#463
