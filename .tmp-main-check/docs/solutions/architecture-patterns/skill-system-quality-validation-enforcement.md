---
title: "Skill system quality: removing [llm] from skill.toml, name-in-keywords rejection, markdown validation"
category: architecture-patterns
date: 2026-04-11
tags: [skills, validation, serde, toml, markdown, breaking-change]
related_issues: [504, 510, 511, 512]
---

# Skill System Quality: Validation, Lifecycle, and Enforcement

## Problem

Three skill system quality gaps:

1. **Dual config surface (#504):** Per-skill LLM provider/model overrides lived in both `skill.toml` `[llm]` section AND the `skill_overrides` DB table (schema v20). The DB was authoritative since v20, making the manifest `[llm]` redundant and confusing.

2. **Redundant keywords (#510):** Skills could list their own name in `[triggers].keywords`, which is meaningless since skills are already matched by name. This wastes keyword slots and creates confusing matching behavior.

3. **Unvalidated markdown (#511):** The `review_skill` builtin handler wrote generated `system_prompt.md` content with only a size-ratio check. Malformed markdown (null bytes, unclosed code blocks, binary content) could corrupt skill prompts.

## Root Cause

1. The `[llm]` section was the original mechanism (pre-v20) for per-skill overrides. When DB-only overrides were added in schema v20, the manifest field wasn't deprecated.
2. No validation existed to catch self-referential keywords.
3. The `review_skill` persist path only checked content size ratio against the source prompt — no structural validation.

## Solution

### #504: `#[serde(skip)]` on `SkillManifest.llm`

**Key insight:** The `LlmOverride` struct and `llm` field must remain on `SkillManifest` as a **runtime-only** field. The `apply_overrides()` method injects DB values into it, and `resolve_skill_llm_override()` reads from it. The change is to stop *deserializing* from TOML while keeping the runtime path.

```rust
// manifest.rs — #[serde(skip)] prevents TOML deserialization
// but field remains writable by apply_overrides() at runtime
#[serde(skip)]
pub llm: LlmOverride,
```

**Why not remove the field entirely?** Because `apply_overrides()` writes to `entry.manifest.llm.provider` and `entry.manifest.llm.model`, and `resolve_skill_llm_override()` reads from `entry.manifest.llm`. Removing the field would require restructuring multiple callers. `#[serde(skip)]` is the minimal change.

**Detection in `validate_skill()`:** Since `#[serde(skip)]` silently ignores `[llm]` during parsing (serde doesn't have `deny_unknown_fields` by default), we detect it via raw TOML parsing:

```rust
if let Ok(raw) = toml::from_str::<toml::Value>(&content)
    && raw.get("llm").is_some()
{
    diags.push(SkillDiagnostic::fail(
        "[llm] section is no longer supported..."
    ));
}
```

**Why `toml::from_str` not `.parse()`?** In `toml` 0.9, `FromStr` for `Value` may not be implemented. Other code in the same file uses `toml::from_str::<toml::Value>()` — follow the existing pattern.

### #510: Name-in-keywords rejection

Exact case-insensitive match only. Partial matches are fine (skill `web-search` with keyword `search` is allowed).

```rust
let name_lower = manifest.skill.name.to_ascii_lowercase();
for kw in &manifest.triggers.keywords {
    if kw.to_ascii_lowercase() == name_lower {
        diags.push(SkillDiagnostic::fail(...));
        break; // one diagnostic is enough
    }
}
```

Added to both `validate_skill()` and `create_skill` tool.

### #511: Lightweight markdown validation

`validate_markdown_content()` in `skills/mod.rs` checks:
1. Empty/whitespace-only content
2. Null bytes (binary data)
3. Control characters (except `\n`, `\r`, `\t`)
4. Unclosed code fences (odd triple-backtick count)

No heavyweight dependencies (no pulldown-cmark/comrak). Wired into:
- `review_skill` persist path (rejects before writing — `ToolOutput::error`)
- `validate_skill()` for existing `system_prompt.md` files (warns, doesn't fail — existing skills shouldn't break)

## Prevention

- **New skill manifest fields:** When adding new TOML fields that become DB-only later, use `#[serde(skip)]` and a raw TOML check in `validate_skill()` to reject the deprecated key with a migration message.
- **Validation consistency:** Any constraint enforced in `validate_skill()` should also be enforced in `create_skill` (and `update_skill` if applicable) to prevent creating invalid skills.
- **Markdown validation scope:** Keep it lightweight — markdown is intentionally permissive. Only catch clear corruption, not stylistic issues.

## Key Files

- `crates/mika-agent/src/skills/manifest.rs` — `#[serde(skip)]` on `llm` field
- `crates/mika-agent/src/skills/mod.rs` — `validate_markdown_content()` helper
- `crates/mika-agent/src/skills/index.rs` — `validate_skill()` additions
- `crates/mika-agent/src/skills/builtin_handlers.rs` — `review_skill` markdown gate
- `crates/mika-agent/src/tools/create_skill.rs` — name-in-keywords check
