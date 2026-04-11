---
title: "Skill system quality: validation, lifecycle, and enforcement improvements"
type: feat
status: active
date: 2026-04-11
---

# Skill System Quality: Validation, Lifecycle, and Enforcement Improvements

## Overview

Umbrella implementation for three tightly coupled sub-issues (#504, #510, #511) that tighten the skill system's validation, lifecycle management, and quality gates. All three changes are self-contained within `mika/` — no cross-repo changes are required in this PR (marketplace skills in `mika-skills/` will be fixed separately once CI enforces the new rules).

## Problem Statement

1. **Dual config surface (#504):** Provider/model overrides live in both `skill.toml` `[llm]` and the `skill_overrides` DB table. This creates confusion about source of truth and complicates the resolution chain (DB > manifest > agent default). The DB is the authoritative source since schema v20 — the manifest `[llm]` section is redundant baggage.

2. **Redundant keywords (#510):** Skills can list their own name in `[triggers].keywords`, which is meaningless — skills are already matched by name. This wastes a keyword slot and creates confusing matching behavior.

3. **Unvalidated markdown (#511):** The `review_skill` builtin handler writes generated `system_prompt.md` content to disk with only a size-ratio check. Malformed markdown (unclosed code blocks, broken frontmatter, binary content) can corrupt skill prompts.

## Proposed Solution

### Phase 1: Remove `[llm]` from skill.toml (#504)

**Key architectural insight:** The `LlmOverride` struct and the `llm` field on `SkillManifest` must be preserved as a **runtime-only** field. The `apply_overrides()` method in `skills/mod.rs` injects DB values into `entry.manifest.llm`, and `resolve_skill_llm_override()` in `agent.rs` reads from it. The change is to stop *deserializing* `[llm]` from TOML while keeping the runtime injection path intact.

#### Changes

**`crates/mika-agent/src/skills/manifest.rs`:**
- Add `#[serde(skip)]` to the `llm` field on `SkillManifest`. This prevents TOML deserialization but preserves the field for runtime use by `apply_overrides()`.
- Keep `LlmOverride` struct unchanged — it's used by `apply_overrides()`, `resolve_skill_llm_override()`, and the `SkillOverride` DB flow.
- Update doc comments to clarify `llm` is runtime-only (populated from DB via `apply_overrides()`).

**`crates/mika-agent/src/skills/index.rs` — `validate_skill()`:**
- Replace the current `[llm]` validation block (lines 647–676) with a **rejection check**: parse the raw TOML as `toml::Value`, check for `["llm"]` key presence, and emit a `SkillDiagnostic::fail` if found:
  ```
  "[llm] section is no longer supported in skill.toml. Use `mika skills llm <name> set <provider>/<model>` to configure per-skill LLM overrides (stored in DB)."
  ```
- This raw-TOML check runs *before* `SkillManifest` deserialization (which now skips `[llm]`), so even a skill.toml with `[llm]` will parse successfully but validation will flag it.

**`crates/mika-agent/src/skills/index.rs` — `warn_missing_llm_api_keys()`:**
- This function checks `entry.manifest.llm.provider` which is now only populated from DB overrides. Its logic remains correct — it still warns about missing API keys for skills with provider overrides. No changes needed except updating the doc comment to clarify the field source is DB-only.

**`crates/mika-agent/src/validate.rs`:**
- The agent-level validation at line 141 checks `entry.manifest.llm.provider` for always-on cross-provider warnings. After this change, this will only trigger for DB-overridden skills — which is correct behavior. Update the comment to clarify.

**`crates/mika-agent/src/skills/mod.rs` — `apply_overrides()`:**
- No changes needed. The method already writes to `entry.manifest.llm.provider` and `entry.manifest.llm.model` from DB values. With `#[serde(skip)]`, the field starts as `Default::default()` (empty) and only contains DB values — exactly what we want.

**`crates/mika-agent/src/agent.rs` — `resolve_skill_llm_override()`:**
- No changes needed. It reads `entry.manifest.llm` which now exclusively contains DB-injected values. Same-provider short-circuit and conflict detection logic remain correct.

**`crates/mika-agent/src/skills/builtin_handlers.rs`:**
- No changes needed. The `review_skill` handler doesn't reference `[llm]`.

**`crates/mika-agent/src/tools/create_skill.rs`:**
- Verify the `create_skill` tool does NOT write `[llm]` to generated `skill.toml` files. Current code uses `toml::to_string_pretty()` on the manifest — with `#[serde(skip)]`, the `llm` field won't be serialized. ✅ No changes needed.

**`crates/mika-cli/src/commands/skills.rs`:**
- The `mika skills llm` subcommand already manages DB overrides exclusively. No changes needed for the create flow (it doesn't inject `[llm]`).

**`crates/mika-agent/src/bundled_skills.rs`:**
- Verify no bundled skill templates contain `[llm]` sections. Current exploration confirms none do. ✅ No changes needed.

**Tests:**
- Update manifest parse tests that include `[llm]` sections:
  - `test_parse_llm_provider_and_model` → verify `[llm]` is now ignored (field stays empty)
  - `test_parse_llm_provider_only`, `test_parse_llm_model_only`, `test_parse_empty_llm_section` → same
  - `test_llm_section_serializes_clean` → verify `[llm]` is not emitted on serialization
  - `test_constraints_coexists_with_llm_section`, `test_context_coexists_with_constraints_and_llm` → update to remove `[llm]` from TOML, test coexistence without it
- Add new test: `test_validate_skill_rejects_llm_section` — create a skill.toml with `[llm]`, run `validate_skill()`, assert FAIL diagnostic.
- Update `apply_overrides` tests — these already test DB injection and should pass unchanged.
- Update `resolve_skill_llm_override` tests that set `entry.manifest.llm` directly — these simulate DB-injected values and should pass unchanged.

### Phase 2: Forbid skill name in keywords (#510)

#### Changes

**`crates/mika-agent/src/skills/index.rs` — `validate_skill()`:**
- After parsing `SkillManifest` (after step 3), add a new validation step:
  ```rust
  // 3c. Reject skill name in keywords
  let name_lower = manifest.skill.name.to_ascii_lowercase();
  for kw in &manifest.triggers.keywords {
      if kw.to_ascii_lowercase() == name_lower {
          diags.push(SkillDiagnostic::fail(format!(
              "skill name '{}' appears in [triggers].keywords — this is redundant \
               (skills are already matched by name). Remove it from keywords.",
              manifest.skill.name
          )));
          break; // one diagnostic is enough
      }
  }
  ```
- Only exact full-name match (case-insensitive). Partial matches are fine — e.g., skill `web-search` can have keyword `search`.

**`crates/mika-agent/src/tools/create_skill.rs`:**
- In the validation section (before writing skill.toml), add the same check:
  ```rust
  let name_lower = name.to_ascii_lowercase();
  if keywords.iter().any(|k| k.to_ascii_lowercase() == name_lower) {
      return ToolOutput::error(format!(
          "Skill name '{}' must not appear in keywords — skills are already matched by name.",
          name
      ));
  }
  ```

**Tests:**
- Add `test_validate_skill_rejects_name_in_keywords` — skill.toml with name in keywords, assert FAIL.
- Add `test_validate_skill_name_in_keywords_case_insensitive` — skill named `Web-Search` with keyword `web-search`, assert FAIL.
- Add `test_validate_skill_partial_name_in_keywords_ok` — skill `web-search` with keyword `search`, assert no FAIL.
- Add `test_create_skill_rejects_name_in_keywords` — create_skill tool test.

### Phase 3: Markdown validation for review_skill (#511)

#### Design Decision: Lightweight validation

Full AST-based markdown validation (pulldown-cmark, comrak) is overkill. The goal is to catch:
1. **Binary/non-text content** — null bytes, control characters
2. **Unclosed code blocks** — odd number of triple-backtick fences
3. **Empty or whitespace-only content**

This avoids adding a heavy dependency (pulldown-cmark is 200KB+) while catching the most common corruption patterns.

#### Changes

**New helper function in `crates/mika-agent/src/skills/builtin_handlers.rs`:**
```rust
/// Lightweight markdown well-formedness check.
/// Returns `Ok(())` or `Err(description)` for common corruption patterns.
fn validate_markdown_content(content: &str) -> Result<(), String> {
    // 1. Reject empty/whitespace-only
    if content.trim().is_empty() {
        return Err("content is empty or whitespace-only".to_string());
    }
    // 2. Reject binary content (null bytes, excessive control chars)
    if content.bytes().any(|b| b == 0) {
        return Err("content contains null bytes — likely binary data".to_string());
    }
    let control_count = content.bytes().filter(|&b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t').count();
    if control_count > 0 {
        return Err(format!(
            "content contains {} control character(s) — likely corrupted",
            control_count
        ));
    }
    // 3. Check for unclosed code fences
    let fence_count = content.lines().filter(|l| l.trim_start().starts_with("```")).count();
    if fence_count % 2 != 0 {
        return Err(format!(
            "content has {} code fence(s) — odd count suggests an unclosed code block",
            fence_count
        ));
    }
    Ok(())
}
```

**`crates/mika-agent/src/skills/builtin_handlers.rs` — `review_skill()` persist path:**
- After the truncation guard (line 1131) and before the overwrite guard (line 1134), add:
  ```rust
  if let Err(reason) = validate_markdown_content(&body) {
      return ToolOutput::error(format!(
          "Generated prompt fails markdown validation: {reason}. \
           Fix the content and re-call review_skill.",
      ));
  }
  ```

**`crates/mika-agent/src/skills/index.rs` — `validate_skill()`:**
- After the prompt size checks, add a markdown validation step for `system_prompt.md`:
  ```rust
  // Check root system_prompt.md markdown well-formedness
  if let Ok(content) = std::fs::read_to_string(skill_dir.join("system_prompt.md")) {
      if let Err(reason) = validate_markdown_content(&content) {
          diags.push(SkillDiagnostic::warn(format!(
              "system_prompt.md: {reason}"
          )));
      }
  }
  ```
- Also check generated variant prompts in `generated/` subdirectories.
- Note: use `warn` not `fail` for existing files — we don't want to break existing skills, just flag issues.

**Move `validate_markdown_content` to a shared location** if both `builtin_handlers.rs` and `index.rs` need it. Options:
- Add it to `skills/mod.rs` as a `pub(crate)` function
- Or define it in `builtin_handlers.rs` and `pub(crate)` export it, importing in `index.rs`

**Tests:**
- `test_validate_markdown_content_valid` — normal markdown, assert Ok
- `test_validate_markdown_content_empty` — empty string, assert Err
- `test_validate_markdown_content_null_bytes` — content with `\0`, assert Err
- `test_validate_markdown_content_control_chars` — content with `\x01`, assert Err
- `test_validate_markdown_content_unclosed_fence` — odd fence count, assert Err
- `test_validate_markdown_content_balanced_fences` — even fence count, assert Ok
- `test_review_skill_rejects_invalid_markdown` — integration test via `review_skill()`
- `test_validate_skill_warns_bad_markdown` — validate_skill with corrupted system_prompt.md

## Acceptance Criteria

### #504 — Remove [llm] from skill.toml
- [x] `#[serde(skip)]` on `SkillManifest.llm` prevents TOML deserialization
- [x] `validate_skill()` emits FAIL for skill.toml files containing `[llm]`
- [x] `apply_overrides()` still injects DB values into `manifest.llm` at runtime
- [x] `resolve_skill_llm_override()` works unchanged with DB-only values
- [x] `create_skill` tool does not emit `[llm]` in generated skill.toml
- [x] Existing manifest tests updated; new rejection test added
- [x] `cargo test` passes

### #510 — Forbid skill name in keywords
- [x] `validate_skill()` emits FAIL when skill name appears in keywords (case-insensitive)
- [x] `create_skill` tool rejects name-in-keywords at creation time
- [x] Partial name matches are allowed (e.g., skill `web-search` with keyword `search`)
- [x] Tests cover exact match, case-insensitive match, and partial-no-match

### #511 — Markdown validation
- [x] `validate_markdown_content()` catches empty, binary, control chars, unclosed fences
- [x] `review_skill` persist path rejects invalid markdown before writing
- [x] `validate_skill()` warns on existing system_prompt.md with bad markdown
- [x] No new heavy dependencies (no pulldown-cmark/comrak)
- [x] Tests cover all validation cases

### All
- [x] `cargo clippy` clean
- [x] `cargo test` passes
- [x] CLAUDE.md updated if needed

## Implementation Order

1. **Phase 2 (#510)** — Smallest change, no ripple effects. Add name-in-keywords check to `validate_skill()` and `create_skill`.
2. **Phase 3 (#511)** — Self-contained. Add `validate_markdown_content()`, wire into `review_skill` and `validate_skill()`.
3. **Phase 1 (#504)** — Largest change with most test updates. Add `#[serde(skip)]`, update `validate_skill()`, update tests.

This order minimizes conflicts between phases.

## Sources

- Issue #512 (umbrella): Skill system quality
- Issue #504: Remove [llm] from skill.toml
- Issue #510: Forbid skill name in keywords
- Issue #511: Markdown validation for review_skill
- `crates/mika-agent/src/skills/manifest.rs` — `LlmOverride` struct, `SkillManifest`
- `crates/mika-agent/src/skills/index.rs` — `validate_skill()`, `warn_missing_llm_api_keys()`
- `crates/mika-agent/src/skills/mod.rs` — `apply_overrides()`
- `crates/mika-agent/src/agent.rs` — `resolve_skill_llm_override()`
- `crates/mika-agent/src/skills/builtin_handlers.rs` — `review_skill()` handler
- `crates/mika-agent/src/tools/create_skill.rs` — `CreateSkillTool`
- `crates/mika-agent/src/validate.rs` — agent-level validation
