---
module: mika-agent/skills, mika-cli, mika-agent/server, dashboard
tags: [skills, variants, validation, management, cli, dashboard]
problem_type: feature
issue: "#541"
---

# Skill Variant Management Tooling

## Problem

Per-provider/per-model prompt variants are a first-class skill extension mechanism, but no tooling existed to detect drift, validate integrity, observe staleness, or gate promotion. The Sprint 2026-04-11 hot-patch chain deleted all 4 existing variants because a minimax self-dev variant silently carried an outdated unconditional `{"action":"allow"}` directive — the base had been fixed but the variant was never updated.

## Solution

Added a comprehensive variant management layer across all three surfaces:

### Shared Module (`crates/mika-agent/src/skills/variants.rs`)

- **`VariantMetadata`** struct with provider, model, source tier, size, staleness
- **`VariantSource`** enum: `HandAuthored`, `Generated`, `Experimental`
- **`RESERVED_VARIANT_DIRS`** constant (`["generated", "experimental"]`) for defense-in-depth exclusion from provider scanning
- **`scan_all_variants()`** — walks all three tiers to produce metadata (separate from runtime `scan_provider_variants` which loads into memory)
- **4-rule validation gate**: size limit (with MIN_VARIANT_RATIO), required sections (configurable via `[variants]` in skill.toml), markdown well-formedness, tool reference lint (warn-only)
- **`diff_variant()`** — frequency-map-based line counting with structural/content/cosmetic classification
- **`reflect_skill()`** — pure computation producing per-variant impact assessments with recommendations
- **`promote_variant()`** — copy-then-delete with hand-authored shadow check, validation gate, and expected-files-only cleanup

### `skill.toml` `[variants]` Section

```toml
[variants]
required_sections = ["Tools", "Constraints"]
max_prompt_size = 32768
```

Backward-compatible via `#[serde(default)]` on `VariantsConfig` in `SkillManifest`.

### `experimental/` Directory Convention

- `experimental/<provider>/<model>/system_prompt.md` is a quarantined playground excluded from runtime resolution
- Defense-in-depth: explicit `RESERVED_VARIANT_DIRS` check in `scan_provider_variants()` (before the existing `ProviderKind` parse which also excludes it)
- Only reachable in production via `promote` which runs validation first

## Key Decisions

1. **Staleness uses mtime comparison** — simple and filesystem-native. Known limitation: git operations normalize timestamps. Git-based staleness deferred to v2.
2. **Diff uses frequency maps** — not sets. Duplicate lines (blank lines, bullets) are counted correctly.
3. **Headings matched at any level** — `#` through `######` all count for required-sections and structural diff classification.
4. **Promote blocks when hand-authored variant exists** — the hand-authored variant would shadow the promoted generated variant at runtime, making the promotion invisible.
5. **Variant module is pure computation** — no DB access, no async. All data from filesystem reads and `SkillManifest` fields.
6. **`skills_dirty` flag after promote** — server sets `AgentState.skills_dirty` to `true` so the next message handler reloads the registry.

## Patterns Worth Reusing

- **`RESERVED_VARIANT_DIRS` defense-in-depth**: when adding new reserved directory names alongside existing implicit guards (like `ProviderKind` parse failure), add an explicit constant check first so future refactors don't accidentally bypass the guard.
- **Copy-then-delete with expected-files-only cleanup**: when promoting/moving files, delete only the files you expect (`system_prompt.md`, `skill.toml`), then `remove_dir` if empty. Never `remove_dir_all` — unexpected files should be preserved with a warning.
- **`VariantsConfig` backward-compatible manifest extension**: use `#[serde(default)]` on a new struct field in `SkillManifest` so existing skill.toml files parse without changes.
