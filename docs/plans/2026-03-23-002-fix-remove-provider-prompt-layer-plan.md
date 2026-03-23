---
title: "fix: Remove provider-level prompt layer from skill variant resolution"
type: fix
status: completed
date: 2026-03-23
---

# Remove Provider-Level Prompt Layer from Skill Variant Resolution

## Overview

PR #247 (`feat-246-skill-variant-model-granularity`) has two problems:
1. It bundled unrelated `.claude/commands/` changes that belong in a separate PR
2. The provider-level prompt layer (`{provider}/system_prompt.md`) is architecturally wrong — gpt-4o and gpt-5 are both "openai" but need different prompts. Only model-level variants make sense for prompt content.

The resolution hierarchy becomes: `{provider}/{model}/system_prompt.md` -> `system_prompt.md` (root). No provider-level prompt.

Provider-level **overrides** (`timeout_secs`, `max_prompt_size`) are retained — these are numeric config, not prompt content, and are legitimately shared across models of the same provider.

## Problem Statement / Motivation

The three-level prompt fallback (`model -> provider -> root`) introduces a semantic error: provider-level prompts assume all models from a provider behave the same way, which is false. OpenAI's gpt-4o and gpt-5 have different capabilities and prompt requirements. The correct abstraction is model-level prompts only.

## Proposed Solution

1. **Revert `.claude/commands/` to main state** — these were unrelated cross-repo command updates
2. **Remove `provider_prompts` field** from `SkillEntry` and all related code paths
3. **Simplify `resolve_prompt()`** to two-level: model-specific -> root
4. **Keep provider-level overrides** for `timeout_secs` and `max_prompt_size` (three-level fallback remains for numeric config)
5. **Update CLI/TUI display** to not show "prompt" for provider-level variants
6. **Update tests** — remove/rewrite tests that reference `provider_prompts`

## Technical Approach

### Phase 1: Revert commands

```bash
git checkout main -- .claude/commands/
```

6 files: mika-issue.md, mika-issues.md, mika-task-review.md, mika-turn-review.md, mika.md, status.md

### Phase 2: Remove `provider_prompts` from core

**`crates/mika-agent/src/skills/index.rs`:**
- Remove `provider_prompts: HashMap<String, String>` from `SkillEntry` struct
- Remove `provider_prompts` from `VariantScanResult` struct
- Update `resolve_prompt()`: model -> root (remove provider prompt lookup at line ~89-91)
- Update `scan_provider_variants()`: stop loading `{provider}/system_prompt.md` as prompt (lines ~942-950), remove from return value
- Update `variant_providers()`: remove `provider_prompts` iteration (lines ~98-100)
- Update `variant_count()`: remove `provider_prompts` iteration (lines ~139-141)
- Update `validate_skill()`: adjust provider-level `has_prompt` check — provider dirs are valid with `skill.toml` overrides or model subdirectories, `system_prompt.md` at provider level is superfluous (warn but don't fail)
- Update `scan_skills_dir()` call site: remove `provider_prompts` from `SkillEntry` construction

**`crates/mika-agent/src/skills/matcher.rs`:**
- Remove `provider_prompts: HashMap::new()` from test helper `SkillEntry` construction

**`crates/mika-agent/src/skills/mod.rs`:**
- Remove `provider_prompts: HashMap::new()` from `SkillRegistry::register()`

### Phase 3: Update CLI/TUI display

**`crates/mika-cli/src/commands/skills.rs`:**
- `show_skill_detail()`: remove `provider_prompts.contains_key()` check for showing "prompt" badge

**`crates/mika-cli/src/tui/commands/handlers.rs`:**
- Update variant display if it references provider prompts

### Phase 4: Update tests

**Tests to remove:**
- `test_inject_skills_uses_provider_prompt` (agent.rs) — tests removed feature
- `test_inject_skills_model_falls_back_to_provider` (agent.rs) — tests removed fallback

**Tests to rewrite:**
- `test_resolve_prompt_model_level` (index.rs) — remove `provider_prompts` insertion
- `test_variant_count` (index.rs) — remove `provider_prompts` from count logic
- `test_variant_count_includes_models` (index.rs) — remove `provider_prompts` references
- `test_inject_skills_falls_back_to_root` (agent.rs) — remove `provider_prompts` field
- `test_inject_skills_uses_model_prompt` (agent.rs) — remove `provider_prompts` alongside model

**Tests to update field only (remove `provider_prompts: HashMap::new()`):**
- ~15 tests in index.rs that construct `SkillEntry` with the field
- `make_skill_entry` helper in agent.rs

**Scan tests (index.rs) — update assertions:**
- `test_scan_with_model_variant_prompt` — currently asserts `provider_prompts.get("anthropic")` exists; remove assertion
- `test_scan_multiple_model_variants` — asserts `provider_prompts.len() == 2`; remove assertion

### Phase 5: Update documentation

- `docs/skills.md` — update three-level fallback description to two-level for prompts
- `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md` — update to reflect removal
- `CLAUDE.md` — update skills description

## Acceptance Criteria

- [x] `.claude/commands/` files match main: `git diff main -- .claude/commands/` is empty
- [x] `provider_prompts` field removed from `SkillEntry` and `VariantScanResult`
- [x] `resolve_prompt()` uses two-level fallback: model -> root
- [x] `effective_timeout()` retains three-level fallback: model -> provider -> root
- [x] Provider directories still scanned for model subdirectories and `skill.toml` overrides
- [x] CLI/TUI display updated for provider variants without prompts
- [x] `cargo test -p mika-agent` passes
- [x] `cargo test -p mika-cli` passes
- [x] `cargo clippy -p mika-agent -p mika-cli` has no warnings

## Critical Files

- `crates/mika-agent/src/skills/index.rs` — main change
- `crates/mika-agent/src/agent.rs` — test updates
- `crates/mika-agent/src/skills/matcher.rs` — field removal
- `crates/mika-agent/src/skills/mod.rs` — field removal
- `crates/mika-cli/src/commands/skills.rs` — CLI display
- `crates/mika-cli/src/tui/commands/handlers.rs` — TUI display
- `.claude/commands/*` — revert to main

## Sources

- Pre-written plan: `~/.claude/plans/abstract-scribbling-crane.md`
- Solution doc: `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md`
- Related: #246, #241, #239
