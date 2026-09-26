---
title: Per-provider and per-model skill variant directories
category: architecture-patterns
tags: [skills, llm-providers, prompt-variants, multi-provider, skill-manifest, model-variants]
date: 2026-03-22
updated: 2026-03-23
severity: low
component: mika-agent
related_issues: ["#241", "#239", "#246"]
---

# Per-Provider and Per-Model Skill Variant Directories

## Problem

Skills have a single `system_prompt.md` shared across all LLM providers and models. Prompts written for Claude (Anthropic) behave differently on OpenAI, Groq, Mistral, etc. Even within the same provider, different models (e.g., Claude Sonnet 4.6 vs Opus 4.6, MiniMax M2.7 vs M2.5) may need different prompt guidance. There was no mechanism to ship model-tuned prompt variants alongside the root prompt.

## Solution

Variant files live in a two-level directory hierarchy under the skill root: `{provider}/` for provider-level overrides and `{provider}/{model}/` for model-level variants. Provider directories are named after `ProviderKind::config_prefix()` values (e.g., `anthropic/`, `openai/`). Model directories within use the model name with slashes replaced by `--` (via `sanitize_model_dir_name()`, e.g., `anthropic--claude-sonnet-4` for OpenRouter's `anthropic/claude-sonnet-4`).

The skill scanner eagerly loads all variants at startup into three `HashMap` fields on `SkillEntry`:

- `provider_overrides: HashMap<String, ProviderSkillFields>` — provider-level sparse manifest field overrides (`timeout_secs`, `max_prompt_size`)
- `model_prompts: HashMap<String, String>` — model-specific prompt content (keyed by `"{provider}/{sanitized_model}"`)
- `model_overrides: HashMap<String, ProviderSkillFields>` — model-level sparse manifest field overrides

At prompt injection time, `inject_skills_and_resolve_tools()` takes both `provider_name` and `model_name` parameters. Resolution uses two-level fallback via `SkillEntry::resolve_prompt()`:

1. Model-specific prompt (`model_prompts["{provider}/{sanitized_model}"]`) → most specific
2. Root `prompt_snippet` → fallback

Provider-level prompts are intentionally **not** supported — models from the same provider (e.g., gpt-4o vs gpt-5) have different capabilities and prompt requirements. Provider-level overrides for numeric config (`timeout_secs`, `max_prompt_size`) remain useful and follow a three-level fallback via `effective_timeout()`: model → provider → root.

### Key Design Decisions

1. **Eager loading:** All variants (provider and model) loaded at scan time. No filesystem access at request time. Runtime `/provider` and `/model` switching works without re-scanning.
2. **Sparse overrides only:** Variant `skill.toml` files (at both levels) contain only fields that differ. Identity fields (`name`, `description`, `version`, `triggers`) cannot be overridden — they define the skill, not its variant behavior.
3. **Recognition by `ProviderKind::from_str()`:** Only subdirectories matching a known provider name are treated as provider variants. `handlers/`, `.git/`, etc. are silently ignored — fully backward compatible.
4. **Model directories are freeform:** Any non-dotdir subdirectory within a provider directory is treated as a model variant. No validation against known model names — models change frequently.
5. **Slash sanitization:** `sanitize_model_dir_name()` replaces `/` with `--` for models with slashes in their names (common with OpenRouter). Applied at both scan time and lookup time.
6. **Two-level nesting maximum:** Provider/model is the deepest nesting supported. Subdirectories within model directories trigger validation warnings.
7. **`always_on` not overridable per-variant:** Matching uses root `always_on` value only. Avoids complexity of skills appearing/disappearing based on provider or model.
8. **Tools/handlers not overridable:** `tools.json` and handler scripts must remain consistent across all variants. If a provider/model needs different tool schemas, that's a different skill.
9. **No provider-level prompts:** Provider directories hold overrides (`skill.toml`) and model subdirectories only. Provider-level `system_prompt.md` files are ignored at scan time and warned about during validation.

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | `ProviderSkillOverride`, `ProviderSkillFields` types (unchanged — reused for model overrides) |
| `crates/mika-agent/src/skills/index.rs` | `SkillEntry` gains `model_prompts`, `model_overrides` fields; `sanitize_model_dir_name()` function; `resolve_prompt(provider, model)`, `variant_models(provider)` methods; `effective_timeout()` and `variant_count()` updated; `scan_provider_variants()` scans model subdirs; `validate_skill()` extended for model dir validation |
| `crates/mika-agent/src/agent.rs` | `inject_skills_and_resolve_tools()` + `max_skill_timeout()` gain `model_name` parameter; all 3 call sites updated; prompt resolution uses `resolve_prompt()` |
| `crates/mika-cli/src/commands/skills.rs` | Info shows model variants nested under providers with tree-style `└─` indentation |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `/skills` handler shows model count per provider |

## Prevention / Best Practices

- When adding a new `ProviderKind` variant, ensure its `config_prefix()` value is unique and lowercase — it doubles as the variant directory name.
- Skill authors should always include a root `system_prompt.md` as fallback for models without dedicated variants.
- For models with slashes in their names (OpenRouter convention), use `--` as the separator in directory names (e.g., `anthropic--claude-sonnet-4` for model `anthropic/claude-sonnet-4`).
- Provider directories need at least a `skill.toml` override or model subdirectories to be valid — empty provider dirs or dirs with only a `system_prompt.md` trigger a warning. Use `mika skills validate` to catch these.
- The `validate_skill()` function includes typo detection for provider subdirectory names that are close to but don't match known providers.
- Validation warns about subdirectories deeper than the model level — only two levels of nesting are supported.
