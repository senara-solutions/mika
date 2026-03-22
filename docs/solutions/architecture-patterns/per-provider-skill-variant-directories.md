---
title: Per-provider skill variant directories
category: architecture-patterns
tags: [skills, llm-providers, prompt-variants, multi-provider, skill-manifest]
date: 2026-03-22
severity: low
component: mika-agent
related_issues: ["#241", "#239"]
---

# Per-Provider Skill Variant Directories

## Problem

Skills have a single `system_prompt.md` shared across all LLM providers. Prompts written for Claude (Anthropic) behave differently on OpenAI, Groq, Mistral, etc. There was no mechanism to ship provider-tuned prompt variants alongside the root prompt, forcing skill authors to write lowest-common-denominator prompts or accept degraded behavior on non-primary providers.

## Solution

Provider-specific files live in subdirectories named after `ProviderKind::config_prefix()` values (e.g., `anthropic/`, `openai/`, `groq/`). The skill scanner eagerly loads all variants at startup into two `HashMap` fields on `SkillEntry`:

- `provider_prompts: HashMap<String, String>` — provider-specific prompt content
- `provider_overrides: HashMap<String, ProviderSkillFields>` — sparse manifest field overrides (`timeout_secs`, `max_prompt_size`)

At prompt injection time, `inject_skills_and_resolve_tools()` takes a `provider_name: &str` parameter and resolves: provider-specific prompt → root `prompt_snippet` fallback.

### Key Design Decisions

1. **Eager loading:** All variants loaded at scan time. No filesystem access at request time. Runtime `/provider` switching works without re-scanning.
2. **Sparse overrides only:** Provider `skill.toml` files contain only fields that differ. Identity fields (`name`, `description`, `version`, `triggers`) cannot be overridden — they define the skill, not its provider behavior.
3. **Recognition by `ProviderKind::from_str()`:** Only subdirectories matching a known provider name are treated as variants. `handlers/`, `.git/`, etc. are silently ignored — fully backward compatible.
4. **`always_on` not overridable per-provider:** Matching uses root `always_on` value only. Avoids complexity of skills appearing/disappearing based on provider.
5. **Tools/handlers not overridable:** `tools.json` and handler scripts must remain consistent. If a provider needs different tool schemas, that's a different skill.

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | `ProviderSkillOverride`, `ProviderSkillFields` types |
| `crates/mika-agent/src/skills/index.rs` | `SkillEntry` fields, `scan_skills_dir()` variant scanning, `validate_skill()` checks, `effective_timeout()` method |
| `crates/mika-agent/src/agent.rs` | `inject_skills_and_resolve_tools()` + `max_skill_timeout()` gain `provider_name` parameter; all 3 call sites updated |
| `crates/mika-cli/src/commands/skills.rs` | `[variants: N]` badge in list, provider details in info |
| `crates/mika-cli/src/tui/commands/handlers.rs` | `/skills` handler variant indicator |

## Prevention / Best Practices

- When adding a new `ProviderKind` variant, ensure its `config_prefix()` value is unique and lowercase — it doubles as the variant directory name.
- Skill authors should always include a root `system_prompt.md` as fallback for providers without dedicated variants.
- Provider variant directories with no `system_prompt.md` and no `skill.toml` are skipped with a warning — use `mika skills validate` to catch empty variants.
- The `validate_skill()` function includes typo detection for subdirectory names that are close to but don't match known providers.
