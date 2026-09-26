# Plan: Per-Provider Skill Variant Directories with Prompt Resolution Fallback

**Issue:** #241
**Branch:** `feat/241/per-provider-skill-variants`
**Date:** 2026-03-22
**Prerequisite:** #239 (per-provider LLM config with `ProviderKind` enum) — merged

## Problem

Skills currently have a single `system_prompt.md` shared across all providers. Prompts written for Claude (Anthropic) behave differently on OpenAI, Groq, Mistral, etc. There is no mechanism to ship provider-tuned prompt variants alongside the root prompt.

## Design

### Directory Layout

Provider-specific files live in subdirectories named after `ProviderKind::config_prefix()` values:

```
skills/web-search/
├── skill.toml              # Root manifest (required)
├── system_prompt.md        # Root prompt (fallback)
├── tools.json              # Tool definitions (NOT overridable per-provider)
├── handlers/               # Handler scripts (NOT overridable per-provider)
├── anthropic/              # Provider variant directory
│   ├── system_prompt.md    # Anthropic-tuned prompt (replaces root)
│   └── skill.toml          # Sparse overrides (optional)
├── openai/
│   └── system_prompt.md    # OpenAI-tuned prompt
└── groq/
    └── system_prompt.md    # Groq-tuned prompt
```

**Identification:** Subdirectories are recognized as provider variants only when their name matches a known `ProviderKind` (via `ProviderKind::from_str()`). Non-matching subdirectories (`handlers/`, `.git/`, etc.) are silently ignored — backward compatible.

### Resolution Order

At prompt injection time, given the active provider name `P`:

1. Check `{skill_dir}/{P}/system_prompt.md` → use if exists
2. Fall back to `{skill_dir}/system_prompt.md` → use if exists
3. No prompt snippet → skill is tools-only (existing behavior)

Same resolution for sparse manifest overrides:
1. Check `{skill_dir}/{P}/skill.toml` → merge overridable fields
2. Use root `skill.toml` values as defaults

### Sparse Manifest Merging

Provider-specific `skill.toml` files contain only fields that differ. A new `ProviderSkillOverride` struct deserializes only the allowed override fields:

```rust
/// Sparse skill.toml overrides from a provider variant directory.
/// Only fields that make sense to vary per-provider are included.
/// Identity fields (name, description, triggers) cannot be overridden.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProviderSkillOverride {
    #[serde(default)]
    pub skill: ProviderSkillFields,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProviderSkillFields {
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub always_on: Option<bool>,
    #[serde(default)]
    pub max_prompt_size: Option<u64>,
}
```

Example `anthropic/skill.toml`:
```toml
[skill]
timeout_secs = 60
```

**What CANNOT be overridden per-provider:** `name`, `description`, `version`, `dependencies`, `[triggers]`, `tools.json`, handler scripts. These are identity/structural and must remain consistent.

### Data Model

**Not** a new field for each possible provider. Instead, a single `HashMap` approach:

```rust
pub struct SkillEntry {
    // ... existing fields ...

    /// Provider-specific prompt snippet overrides.
    /// Key = provider name (e.g., "anthropic"), value = prompt content.
    /// Empty map if no variants exist. Populated eagerly at scan time.
    pub provider_prompts: HashMap<String, String>,

    /// Provider-specific manifest field overrides.
    /// Key = provider name, value = sparse override fields.
    /// Empty map if no variants exist.
    pub provider_overrides: HashMap<String, ProviderSkillFields>,
}
```

This approach:
- Loads all variants eagerly at scan time (no filesystem access at request time)
- Supports runtime provider switching (`/provider` command) without re-scanning
- HashMap is empty for skills without variants (zero overhead for existing skills)

### Prompt Injection Change

`inject_skills_and_resolve_tools` gains a `provider_name: &str` parameter:

```rust
fn inject_skills_and_resolve_tools(
    matched: &[&SkillEntry],
    tools: &ToolRegistry,
    system: &mut String,
    provider_name: &str,    // NEW
) -> Vec<ToolDefinition> {
    // ...
    for entry in matched {
        // Resolve prompt: provider-specific > root
        let prompt = entry.provider_prompts
            .get(provider_name)
            .unwrap_or(&entry.prompt_snippet);

        if !prompt.is_empty() {
            write!(system, "\n<context ...>\n## {} Skill\n{}\n</context>\n",
                entry.manifest.skill.name, prompt).unwrap();
        }
        // ... tool resolution unchanged ...
    }
}
```

All 3 call sites pass `llm.provider_name()`:
- `run_agent()` (agent.rs ~line 786)
- `run_silent_agent()` (agent.rs ~line 1477)
- `run_team_agent()` (agent.rs ~line 1693)

### Timeout Override Resolution

When `provider_overrides` exist for the active provider and contain a `timeout_secs`, the `max_skill_timeout()` helper must use it. The cleanest approach is a new method on `SkillEntry`:

```rust
impl SkillEntry {
    /// Effective timeout for a given provider, considering overrides.
    pub fn effective_timeout(&self, provider: &str) -> u64 {
        self.provider_overrides
            .get(provider)
            .and_then(|o| o.timeout_secs)
            .unwrap_or(self.manifest.skill.timeout_secs)
    }
}
```

Similarly for `always_on`:

```rust
impl SkillEntry {
    /// Effective always_on for a given provider, considering overrides.
    pub fn effective_always_on(&self, provider: &str) -> bool {
        self.provider_overrides
            .get(provider)
            .and_then(|o| o.always_on)
            .unwrap_or(self.manifest.skill.always_on)
    }
}
```

**Impact on matching:** `match_skills()` and `always_on_skills()` use `manifest.skill.always_on` for filtering. Provider-level `always_on` overrides introduce a design choice:
- **Option A (Recommended):** Provider overrides do NOT affect matching — matching uses the root `always_on` value. Provider overrides only affect prompt selection and timeout. Rationale: matching happens before we know which prompt variant to use, and it's confusing for a skill to appear/disappear based on provider.
- **Option B:** Thread provider name into `match_skills()`. More complex, less clear benefit.

**Decision:** Option A. `always_on` override in provider variants is **omitted from v1**. Only `timeout_secs` and `max_prompt_size` are supported as overrides. This avoids the matching complexity and keeps the feature focused on prompt variants.

## Implementation Steps

### Step 1: New Types in `manifest.rs`

Add `ProviderSkillOverride` and `ProviderSkillFields` structs. These are purely additive with no impact on existing code.

**File:** `crates/mika-agent/src/skills/manifest.rs`

### Step 2: Extend `SkillEntry` in `index.rs`

Add two new fields to `SkillEntry`:
- `provider_prompts: HashMap<String, String>`
- `provider_overrides: HashMap<String, ProviderSkillFields>`

Both default to empty HashMap. Update all existing `SkillEntry` constructors in tests.

**File:** `crates/mika-agent/src/skills/index.rs`

### Step 3: Scan Provider Variant Directories in `scan_skills_dir()`

After loading the root manifest and prompt, iterate over subdirectories of each skill directory:
1. Try `ProviderKind::from_str(subdir_name)` — if it fails, skip (not a provider variant)
2. If it's a known provider:
   a. Load `{subdir}/system_prompt.md` with size limit → insert into `provider_prompts`
   b. Load `{subdir}/skill.toml` if exists → parse as `ProviderSkillOverride` → insert into `provider_overrides`
   c. Warn if provider `skill.toml` exists but is malformed
   d. Warn if provider directory is empty (no prompt, no override)

Size limits for provider variant files use the same constants as root files.

**File:** `crates/mika-agent/src/skills/index.rs` — in `scan_skills_dir()`, after the `entries.push(...)` block

### Step 4: Add `effective_timeout()` Method to `SkillEntry`

Add a convenience method that resolves timeout with provider override fallback.

**File:** `crates/mika-agent/src/skills/index.rs`

### Step 5: Update `inject_skills_and_resolve_tools()`

Add `provider_name: &str` parameter. Use `provider_prompts.get(provider_name)` with fallback to root `prompt_snippet`. Update all 3 call sites to pass `llm.provider_name()`.

**File:** `crates/mika-agent/src/agent.rs`

### Step 6: Update `max_skill_timeout()`

Thread provider name through `max_skill_timeout()` and use `effective_timeout()` on each entry.

**File:** `crates/mika-agent/src/agent.rs`

### Step 7: Extend `validate_skill()` for Provider Variants

After existing validation, scan for provider subdirectories:
- For each known provider subdir, validate:
  - `system_prompt.md` size against effective limit
  - `skill.toml` parseability as `ProviderSkillOverride`
  - Warn if provider subdir contains `tools.json` (not supported)
  - Warn if provider subdir is empty
- Warn on subdirectories that look like provider names but don't match any known provider (typo detection)

**File:** `crates/mika-agent/src/skills/index.rs` — in `validate_skill()`

### Step 8: CLI `mika skills info` — Show Provider Variants

Extend the `show_skill_detail()` function to display which providers have variants and what they override.

**File:** `crates/mika-cli/src/commands/skills.rs`

### Step 9: CLI `mika skills list` — Optional Variant Indicator

In the standard listing, append a `[variants: N]` badge showing how many provider variants exist.

**File:** `crates/mika-cli/src/commands/skills.rs`

### Step 10: TUI `/skills` Handler — Variant Indicator

Mirror the CLI variant indicator in the TUI `/skills` slash command output.

**File:** `crates/mika-cli/src/tui/commands/handlers.rs`

### Step 11: Tests

#### Unit Tests (index.rs)

1. **`test_scan_with_provider_variant_prompt`** — Skill with `anthropic/system_prompt.md` → `provider_prompts` populated
2. **`test_scan_with_provider_variant_override`** — Skill with `anthropic/skill.toml` containing `timeout_secs` → `provider_overrides` populated
3. **`test_scan_ignores_non_provider_subdirs`** — `handlers/` subdir is not treated as provider variant
4. **`test_scan_empty_provider_dir_skipped`** — Provider subdir with no files → not in maps
5. **`test_scan_malformed_provider_override_warned`** — Bad TOML in provider `skill.toml` → skipped with warning
6. **`test_effective_timeout_with_override`** — Provider override returns provider timeout
7. **`test_effective_timeout_without_override`** — No override returns root timeout
8. **`test_effective_timeout_unknown_provider`** — Unknown provider returns root timeout

#### Unit Tests (agent.rs)

9. **`test_inject_skills_uses_provider_prompt`** — With "anthropic" provider, injects `anthropic/system_prompt.md` content
10. **`test_inject_skills_falls_back_to_root`** — With "groq" provider (no variant), injects root `system_prompt.md`
11. **`test_inject_skills_no_prompt_at_all`** — Skill with no root prompt and no variant → no context block

#### Unit Tests (manifest.rs)

12. **`test_parse_provider_override_sparse`** — Only `timeout_secs` present, other fields `None`
13. **`test_parse_provider_override_full`** — All overridable fields present
14. **`test_parse_provider_override_empty_skill_section`** — `[skill]` section with no fields → all `None`

#### Integration Tests (validate_skill)

15. **`test_validate_provider_variant_valid`** — Valid provider variant passes
16. **`test_validate_provider_variant_tools_json_warn`** — tools.json in provider dir → warning
17. **`test_validate_provider_subdir_typo_warn`** — "antropic" subdir → typo warning

### Step 12: Update Test Helpers

Update `make_entry` and `make_entry_with_deps` test helper functions in `mod.rs` and `matcher.rs` to include the new `HashMap::new()` fields.

**Files:** `crates/mika-agent/src/skills/mod.rs`, `crates/mika-agent/src/skills/matcher.rs`

## Files Changed

| File | Change Type | Description |
|------|-------------|-------------|
| `crates/mika-agent/src/skills/manifest.rs` | Add | `ProviderSkillOverride`, `ProviderSkillFields` types |
| `crates/mika-agent/src/skills/index.rs` | Modify | `SkillEntry` fields, `scan_skills_dir()` provider scanning, `validate_skill()` variant checks, `effective_timeout()` method |
| `crates/mika-agent/src/skills/mod.rs` | Modify | Test helpers for new fields |
| `crates/mika-agent/src/skills/matcher.rs` | Modify | Test helpers for new fields |
| `crates/mika-agent/src/agent.rs` | Modify | `inject_skills_and_resolve_tools()` signature + logic, `max_skill_timeout()`, 3 call sites |
| `crates/mika-cli/src/commands/skills.rs` | Modify | `list_skills()` variant badge, `show_skill_detail()` variant details |
| `crates/mika-cli/src/tui/commands/handlers.rs` | Modify | `/skills` handler variant indicator |

## Non-Goals (Deferred)

- **Per-provider `tools.json` or handler scripts** — tools/handlers must remain consistent across providers. If a provider needs different tool schemas, that's a different skill.
- **Bundled skill variants** — Bundled skills ship without variants initially. Community/marketplace skills are the primary use case.
- **Per-provider `always_on` override** — Omitted from v1 to avoid matching complexity. Revisit if concrete use cases emerge.
- **`[llm]` section in provider skill.toml** — Mentioned in the issue for future provider-specific LLM config (e.g., different temperature). Not implemented in v1.
- **Automatic provider detection for prompt optimization** — A future feature could auto-generate variants using LLM translation. Not in scope.

## Backward Compatibility

- Skills without provider variant directories work exactly as before (empty HashMaps, no behavior change)
- Existing `handlers/` subdirectories are not mistaken for provider variants (`ProviderKind::from_str("handlers")` returns `Err`)
- No schema changes, no config changes, no new env vars
- No changes to marketplace lock file format
- No changes to skill installation/uninstall flow (provider variant dirs are copied/symlinked with the rest of the skill directory)

## Documentation

- Update `docs/skills.md` with the provider variant directory structure and resolution rules
- Add examples of provider-specific `system_prompt.md` files
- Document the sparse `skill.toml` override format
- Update `docs/configuration.md` if needed (no config changes expected)
